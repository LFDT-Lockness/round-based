//! Protocol in which honest parties reach consensus on its termination status

#![no_std]
#![forbid(unused_crate_dependencies, missing_docs)]

#[cfg(test)]
extern crate std;

extern crate alloc;

mod _unused_deps {
    // We don't use it directly, but we need to enable `serde` feature
    use generic_array as _;
}

use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use round_based::rounds_router::{
    RoundsRouter, round_buf::RoundBufBounded, simple_store::RoundInput,
};
use round_based::{Delivery, Mpc, MpcParty, Outgoing, PartyIndex, ProtocolMessage, SinkExt};

/// Protocol message
#[derive(Clone, Debug, PartialEq, ProtocolMessage, Serialize, Deserialize)]
pub enum Msg {
    /// Termination round
    Termination(TerminationMsg),
}

/// Message from round 1
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminationMsg {
    /// `None` if `sig` was issued by the sender, `Some(i)` if `sig` is relayed from party `i`
    pub relayed_from: Option<PartyIndex>,
    /// Signature confirming successful termination
    pub sig: eddsa::Signature,
}

/// Runs the protocol with consensus on its termination
///
/// Protocol is secure in presence of malicious parties. Malicious party may abort the protocol.
/// However, it is guaranteed that all honest parties either terminate successfully or abort all
/// together. I.e. malicious party cannot cause one honest party to abort, and one other party to
/// terminate successfully. Note that it's acceptable that one subset of honest parties terminate
/// with an error, while other subset hang up indefinitely (the function never returns). It's up to
/// the caller to handle protocol that is stuck.
///
/// Protocol only requires peer-to-peer channels. Parties send multicast messages to all other
/// participants, but whether it's done over peer-to-peer channels or through some other mechanism
/// is not important.
///
/// This pattern can be used in larger protocols, in which it's mandatory for security that
/// honest parties reach consensus on the protocol termination.
///
/// Accepts:
/// - MPC engine `party` that abstracts the network communications
/// - Globally unique `execution_nonce` of this protocol run
/// - This party index `i`
/// - This party secret `signing_key`
/// - List of `verifying_keys` corresponding to each participant \
///   `verifying_key[i]` must correspond to provided `signing_key`
pub async fn consensus_on_termination<M>(
    party: M,
    execution_nonce: &[u8],
    i: PartyIndex,
    signing_key: &eddsa::SigningKey,
    verifying_keys: &[eddsa::VerifyingKey],
) -> Result<(), Error>
where
    M: Mpc<ProtocolMessage = Msg>,
{
    // Validate inputs
    if Some(&signing_key.verifying_key()) != verifying_keys.get(usize::from(i)) {
        return Err(Reason::SigningKeyDoesntMatchVerifyingKey.into());
    }
    if verifying_keys.len() < 2 {
        return Err(Reason::TooFewParties {
            n: verifying_keys.len(),
        }
        .into());
    }
    let n: u16 = verifying_keys
        .len()
        .try_into()
        .map_err(|_| Reason::TooManyParties {
            n: verifying_keys.len(),
        })?;

    let MpcParty { delivery, .. } = party.into_party();
    let (incoming, mut outgoing) = delivery.split();

    // Define rounds
    let mut rounds = RoundsRouter::<Msg>::builder();
    // Buffered round allows the protocol implementation to process incoming messages manually
    // one-by-one, rather than by an implementation of `MessagesStore`
    //
    // Each party sends at most `n` messages in this round: its own signature, and signatures it
    // relays from other parties
    let round1_buf = RoundBufBounded::<TerminationMsg>::new(n, usize::from(n));
    let mut round1 = rounds.add_buffered_round(round1_buf);
    let mut rounds = rounds.listen(incoming);

    // --- The Protocol ---

    // All honest parties terminate successfully. They produce a signature on `msg_to_sign` with
    // domain separation tag `DST` to confirm so.
    let msg_to_sign = sha2::Sha512::new_with_prefix(execution_nonce);
    const DST: &[u8] = b"round-based.example.consensus-on-termination";
    let sig = signing_key
        .sign_prehashed(msg_to_sign.clone(), Some(DST))
        .map_err(Reason::EddsaSign)?;

    // Send the signature to all other parties
    outgoing
        .send(Outgoing::broadcast(Msg::Termination(TerminationMsg {
            relayed_from: None,
            sig,
        })))
        .await
        .map_err(|err| Reason::SendMySig(alloc::boxed::Box::new(err)))?;

    // Receive signatures from other parties
    use round_based::rounds_router::MessagesStore;
    let mut sigs = RoundInput::p2p(i, n);
    while sigs.wants_more() {
        let incoming = rounds
            .next_msg_in(&mut round1)
            .await
            .map_err(|err| Reason::ReceiveSig(alloc::boxed::Box::new(err)))?;
        // Process incoming message. Invalid messages are ignored. Note: we could add identifiable
        // abort to blame parties who send malformed messages, but it's out of scope of this
        // example.
        let sender = if let Some(relayed_from) = incoming.msg.relayed_from {
            // `incoming.sender` relayed message from party with index `relayed_from`
            relayed_from
        } else {
            incoming.sender
        };

        // Did we already receive a message from `relayed_from` party?
        if !sigs.needs_msg_from(sender) {
            // ignore a message that we already received
            continue;
        }

        // Is signature valid?
        let verifying_key =
            verifying_keys
                .get(usize::from(sender))
                .ok_or_else(|| Reason::BugUnknownParty {
                    sender,
                    verifying_keys_len: verifying_keys.len(),
                })?;
        if verifying_key
            .verify_prehashed(msg_to_sign.clone(), Some(DST), &incoming.msg.sig)
            .is_err()
        {
            // ignore invalid signature
            continue;
        }

        // save the signature
        {
            let mut incoming = incoming;
            // We only assume p2p channels, doesn't matter if message was broadcasted
            incoming.msg_type = round_based::MessageType::P2P;
            // If message was relayed, set correct sender index
            incoming.sender = sender;
            sigs.add_message(incoming.map(|m| m.sig))
                .map_err(Reason::BugSaveSig)?;
        }

        // relay received message to all other parties
        outgoing
            .send(Outgoing::broadcast(Msg::Termination(TerminationMsg {
                relayed_from: Some(sender),
                sig: incoming.msg.sig,
            })))
            .await
            .map_err(|err| Reason::RelaySig(alloc::boxed::Box::new(err)))?;
    }

    // When we receive `n` valid signatures from any mix of direct and relayed messages, consensus
    // is reached: all honest parties share the same view of successful protocol termination
    Ok(())
}

/// Protocol error
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] Reason);

type IoError = alloc::boxed::Box<dyn core::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
enum Reason {
    #[error("invalid input: provided signing_key does not match verifying_keys[i]")]
    SigningKeyDoesntMatchVerifyingKey,
    #[error("too few parties: at least 2 parties required, {n} provided")]
    TooFewParties { n: usize },
    #[error("number of parties n={n} overflows u16")]
    TooManyParties { n: usize },
    #[error("bug: signing failed")]
    EddsaSign(eddsa::SignatureError),
    #[error("bug: `relayed_from` index is out of bounds")]
    BugUnknownParty {
        sender: u16,
        verifying_keys_len: usize,
    },
    /// Party couldn't send a message with termination success message containing its own signature
    #[error("send termination confirmation to other parties")]
    SendMySig(#[source] IoError),
    #[error("relay termination confirmation received from one party to others")]
    RelaySig(#[source] IoError),
    #[error("receive termination msg from another party")]
    ReceiveSig(#[source] IoError),
    #[error("bug: saving sig returned an error")]
    BugSaveSig(#[source] round_based::rounds_router::simple_store::RoundInputError),
}

#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use round_based::state_machine::{ProceedResult, StateMachine, wrap_protocol};
    use round_based::{Incoming, MessageType};

    use rand::Rng;

    use super::{Msg, TerminationMsg, consensus_on_termination};

    const A: u16 = 0;
    const B: u16 = 1;
    const C: u16 = 2;

    /// Proceeds the state machine until it either needs one more message or outputs.
    /// Returns messages that the party sent in the meantime.
    fn proceed<S: StateMachine<Msg = Msg>>(party: &mut S) -> (Vec<Msg>, Option<S::Output>) {
        let mut sent = Vec::new();
        loop {
            match party.proceed() {
                ProceedResult::SendMsg(msg) => {
                    assert!(msg.is_broadcast());
                    sent.push(msg.msg)
                }
                ProceedResult::NeedsOneMoreMessage => return (sent, None),
                ProceedResult::Yielded => {}
                ProceedResult::Output(out) => return (sent, Some(out)),
                ProceedResult::Error(err) => panic!("state machine error: {err}"),
            }
        }
    }
    fn incoming(sender: u16, msg: &Msg) -> Incoming<Msg> {
        Incoming {
            id: 0,
            sender,
            msg_type: MessageType::P2P,
            msg: msg.clone(),
        }
    }

    #[test]
    fn simulation() {
        let mut rng = rand_dev::DevRng::new();

        let execution_nonce: [u8; 32] = rng.r#gen();

        const N: u16 = 5;
        let sk = [(); N as usize].map(|_| eddsa::SigningKey::generate(&mut rng));
        let pk = sk.each_ref().map(|sk| sk.verifying_key());

        round_based::sim::run_with_setup(sk, |i, party, sk| async move {
            consensus_on_termination(party, &execution_nonce, i, &sk, &pk).await
        })
        .unwrap()
        .expect_ok()
        .expect_eq();

        std::println!("All parties terminated successfully!");
    }

    /// Malicious party `C` sends its confirmation only to honest party `A`. Honest party `B`
    /// must still terminate successfully as `A` relays the confirmation to it.
    #[test]
    fn malicious_party_sends_confirmation_to_one_party() {
        let mut rng = rand_dev::DevRng::new();

        let execution_nonce: [u8; 32] = rng.r#gen();
        let sk = [(); 3].map(|_| eddsa::SigningKey::generate(&mut rng));
        let pk = sk.each_ref().map(|sk| sk.verifying_key());

        let mut a = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, A, &sk[0], &pk)
        });
        let mut b = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, B, &sk[1], &pk)
        });
        // `c` behaves honestly. The test makes it malicious by not relaying message to party B.
        let mut c = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, C, &sk[2], &pk)
        });

        // Each party starts with sending its own confirmation
        let (a_confirmation, output) = proceed(&mut a);
        assert!(output.is_none() && a_confirmation.len() == 1);
        let (b_confirmation, output) = proceed(&mut b);
        assert!(output.is_none() && b_confirmation.len() == 1);
        let (c_confirmation, output) = proceed(&mut c);
        assert!(output.is_none() && c_confirmation.len() == 1);

        // `A` receives confirmation from `C`, and relays it. `B` never receives it from `C`.
        a.received_msg(incoming(C, &c_confirmation[0])).unwrap();
        let (a_relayed_c, output) = proceed(&mut a);
        assert!(output.is_none());
        assert!(matches!(
            a_relayed_c[..],
            [Msg::Termination(TerminationMsg {
                relayed_from: Some(C),
                ..
            })]
        ));

        // B receives confirmation from A. It still misses confirmation of C.
        b.received_msg(incoming(A, &a_confirmation[0])).unwrap();
        let (_b_relayed_a, output) = proceed(&mut b);
        assert!(output.is_none());

        // A receives confirmation from B. Now it has all confirmations, so it terminates.
        a.received_msg(incoming(B, &b_confirmation[0])).unwrap();
        let (_a_relayed_b, output) = proceed(&mut a);
        output.expect("A must terminate").unwrap();

        // B receives confirmation of C relayed by A, and terminates as well
        b.received_msg(incoming(A, &a_relayed_c[0])).unwrap();
        let (_b_relayed_c, output) = proceed(&mut b);
        output.expect("B must terminate").unwrap();
    }

    /// Malicious party `C` floods honest party `A` with more messages than any honest party
    /// sends. It must not cause `A` to abort: both `A` and `B` terminate successfully.
    #[test]
    fn malicious_party_floods_one_party() {
        let mut rng = rand_dev::DevRng::new();

        let execution_nonce: [u8; 32] = rng.r#gen();
        let sk = [(); 3].map(|_| eddsa::SigningKey::generate(&mut rng));
        let pk = sk.each_ref().map(|sk| sk.verifying_key());

        let mut a = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, A, &sk[0], &pk)
        });
        let mut b = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, B, &sk[1], &pk)
        });
        // `c` behaves honestly. The test makes it malicious by sending its confirmation to `A`
        // many times.
        let mut c = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, C, &sk[2], &pk)
        });

        // Each party starts with sending its own confirmation
        let (a_confirmation, output) = proceed(&mut a);
        assert!(output.is_none() && a_confirmation.len() == 1);
        let (b_confirmation, output) = proceed(&mut b);
        assert!(output.is_none() && b_confirmation.len() == 1);
        let (c_confirmation, output) = proceed(&mut c);
        assert!(output.is_none() && c_confirmation.len() == 1);

        // `C` sends its confirmation to `A` way more times than the limit of messages per party.
        // `A` relays the confirmation once, and ignores the rest.
        let mut a_relayed_c = Vec::new();
        for _ in 0..20 {
            a.received_msg(incoming(C, &c_confirmation[0])).unwrap();
            let (sent, output) = proceed(&mut a);
            assert!(output.is_none(), "A must neither abort nor terminate");
            a_relayed_c.extend(sent);
        }
        assert!(matches!(
            a_relayed_c[..],
            [Msg::Termination(TerminationMsg {
                relayed_from: Some(C),
                ..
            })]
        ));

        // A receives confirmation from B. Now it has all confirmations, so it terminates.
        a.received_msg(incoming(B, &b_confirmation[0])).unwrap();
        let (_a_relayed_b, output) = proceed(&mut a);
        output.expect("A must terminate").unwrap();

        // B receives confirmations of A and C, and terminates as well
        b.received_msg(incoming(A, &a_confirmation[0])).unwrap();
        let (_b_relayed_a, output) = proceed(&mut b);
        assert!(output.is_none());
        b.received_msg(incoming(C, &c_confirmation[0])).unwrap();
        let (_b_relayed_c, output) = proceed(&mut b);
        output.expect("B must terminate").unwrap();
    }

    /// Malicious party `C` sends invalid messages to honest party `A` until it hits the limit of
    /// messages per party, then it sends its confirmation to `A` only. `A` discards the
    /// confirmation, so no one has it: neither `A` nor `B` terminates, and none of them aborts.
    #[test]
    fn malicious_party_hits_the_limit_before_sending_confirmation() {
        let mut rng = rand_dev::DevRng::new();

        let execution_nonce: [u8; 32] = rng.r#gen();
        let sk = [(); 3].map(|_| eddsa::SigningKey::generate(&mut rng));
        let pk = sk.each_ref().map(|sk| sk.verifying_key());
        let n = 3;

        let mut a = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, A, &sk[0], &pk)
        });
        let mut b = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, B, &sk[1], &pk)
        });
        // `c` behaves honestly. The test makes it malicious by sending other messages on its
        // behalf.
        let mut c = wrap_protocol(|party| {
            consensus_on_termination(party, &execution_nonce, C, &sk[2], &pk)
        });

        // Each party starts with sending its own confirmation
        let (a_confirmation, output) = proceed(&mut a);
        assert!(output.is_none() && a_confirmation.len() == 1);
        let (b_confirmation, output) = proceed(&mut b);
        assert!(output.is_none() && b_confirmation.len() == 1);
        let (c_confirmation, output) = proceed(&mut c);
        assert!(output.is_none() && c_confirmation.len() == 1);

        // `C` sends `n` invalid messages to `A`: signature of `B` is not a valid signature of
        // `C`. `A` ignores them.
        for _ in 0..n {
            a.received_msg(incoming(C, &b_confirmation[0])).unwrap();
            let (sent, output) = proceed(&mut a);
            assert!(output.is_none(), "A must neither abort nor terminate");
            assert!(sent.is_empty());
        }

        // Now `C` sends a valid confirmation to `A`, but it's over the limit, so `A` discards it
        // and doesn't relay it to `B`
        a.received_msg(incoming(C, &c_confirmation[0])).unwrap();
        let (sent, output) = proceed(&mut a);
        assert!(output.is_none(), "A must neither abort nor terminate");
        assert!(sent.is_empty());

        // `A` and `B` exchange confirmations and everything they relayed. Both of them miss
        // the confirmation of `C`, so they keep waiting for it.
        a.received_msg(incoming(B, &b_confirmation[0])).unwrap();
        let (a_relayed_b, output) = proceed(&mut a);
        assert!(output.is_none(), "A must neither abort nor terminate");
        assert_eq!(a_relayed_b.len(), 1);

        b.received_msg(incoming(A, &a_confirmation[0])).unwrap();
        let (b_relayed_a, output) = proceed(&mut b);
        assert!(output.is_none(), "B must neither abort nor terminate");
        assert_eq!(b_relayed_a.len(), 1);

        b.received_msg(incoming(A, &a_relayed_b[0])).unwrap();
        let (sent, output) = proceed(&mut b);
        assert!(output.is_none(), "B must neither abort nor terminate");
        assert!(sent.is_empty());

        a.received_msg(incoming(B, &b_relayed_a[0])).unwrap();
        let (sent, output) = proceed(&mut a);
        assert!(output.is_none(), "A must neither abort nor terminate");
        assert!(sent.is_empty());
    }
}
