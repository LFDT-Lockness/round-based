//! Simple protocol in which parties cooperate to generate randomness

#![no_std]
#![forbid(unused_crate_dependencies, missing_docs)]

#[cfg(any(feature = "std", test))]
extern crate std;

extern crate alloc;

mod _unused_deps {
    // We don't use it directy, but we need to enable `serde` feature
    use generic_array as _;
}

use alloc::{vec, vec::Vec};

use serde::{Deserialize, Serialize};
use sha2::{digest::Output, Digest, Sha256};

use round_based::rounds_router::{
    simple_store::{RoundInput, RoundInputError},
    CompleteRoundError, RoundsRouter,
};
use round_based::{Delivery, Mpc, MpcParty, MsgId, Outgoing, PartyIndex, ProtocolMessage, SinkExt};

/// Protocol message
#[derive(Clone, Debug, PartialEq, ProtocolMessage, Serialize, Deserialize)]
pub enum Msg {
    /// Round 1
    CommitMsg(CommitMsg),
    /// Round 2
    DecommitMsg(DecommitMsg),
}

/// Message from round 1
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitMsg {
    /// Party commitment
    pub commitment: Output<Sha256>,
}

/// Message from round 2
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecommitMsg {
    /// Randomness generated by party
    pub randomness: [u8; 32],
}

/// Carries out the randomness generation protocol
pub async fn protocol_of_random_generation<R, M>(
    party: M,
    i: PartyIndex,
    n: u16,
    mut rng: R,
) -> Result<[u8; 32], Error<M::ReceiveError, M::SendError>>
where
    M: Mpc<ProtocolMessage = Msg>,
    R: rand_core::RngCore,
{
    let MpcParty { delivery, .. } = party.into_party();
    let (incoming, mut outgoing) = delivery.split();

    // Define rounds
    let mut rounds = RoundsRouter::<Msg>::builder();
    let round1 = rounds.add_round(RoundInput::<CommitMsg>::broadcast(i, n));
    let round2 = rounds.add_round(RoundInput::<DecommitMsg>::broadcast(i, n));
    let mut rounds = rounds.listen(incoming);

    // --- The Protocol ---

    // 1. Generate local randomness
    let mut local_randomness = [0u8; 32];
    rng.fill_bytes(&mut local_randomness);

    // 2. Commit local randomness (broadcast m=sha256(randomness))
    let commitment = Sha256::digest(local_randomness);
    outgoing
        .send(Outgoing::broadcast(Msg::CommitMsg(CommitMsg {
            commitment,
        })))
        .await
        .map_err(Error::Round1Send)?;

    // 3. Receive committed randomness from other parties
    let commitments = rounds
        .complete(round1)
        .await
        .map_err(Error::Round1Receive)?;

    // 4. Open local randomness
    outgoing
        .send(Outgoing::broadcast(Msg::DecommitMsg(DecommitMsg {
            randomness: local_randomness,
        })))
        .await
        .map_err(Error::Round2Send)?;

    // 5. Receive opened local randomness from other parties, verify them, and output protocol randomness
    let randomness = rounds
        .complete(round2)
        .await
        .map_err(Error::Round2Receive)?;

    let mut guilty_parties = vec![];
    let mut output = local_randomness;
    for ((party_i, com_msg_id, commit), (_, decom_msg_id, decommit)) in commitments
        .into_iter_indexed()
        .zip(randomness.into_iter_indexed())
    {
        let commitment_expected = Sha256::digest(decommit.randomness);
        if commit.commitment != commitment_expected {
            guilty_parties.push(Blame {
                guilty_party: party_i,
                commitment_msg: com_msg_id,
                decommitment_msg: decom_msg_id,
            });
            continue;
        }

        output
            .iter_mut()
            .zip(decommit.randomness)
            .for_each(|(x, r)| *x ^= r);
    }

    if !guilty_parties.is_empty() {
        Err(Error::PartiesOpenedRandomnessDoesntMatchCommitment { guilty_parties })
    } else {
        Ok(output)
    }
}

/// Protocol error
#[derive(Debug, displaydoc::Display)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Error<RecvErr, SendErr> {
    /// Couldn't send a message in the first round
    #[displaydoc("send a message at round 1")]
    Round1Send(#[cfg_attr(feature = "std", source)] SendErr),
    /// Couldn't receive a message in the first round
    #[displaydoc("receive messages at round 1")]
    Round1Receive(
        #[cfg_attr(feature = "std", source)] CompleteRoundError<RoundInputError, RecvErr>,
    ),
    /// Couldn't send a message in the second round
    #[displaydoc("send a message at round 2")]
    Round2Send(#[cfg_attr(feature = "std", source)] SendErr),
    /// Couldn't receive a message in the second round
    #[displaydoc("receive messages at round 2")]
    Round2Receive(
        #[cfg_attr(feature = "std", source)] CompleteRoundError<RoundInputError, RecvErr>,
    ),

    /// Some of the parties cheated
    #[displaydoc("malicious parties: {guilty_parties:?}")]
    PartiesOpenedRandomnessDoesntMatchCommitment {
        /// List of cheated parties
        guilty_parties: Vec<Blame>,
    },
}

/// Blames a party in cheating during the protocol
#[derive(Debug)]
pub struct Blame {
    /// Index of the cheated party
    pub guilty_party: PartyIndex,
    /// ID of the message that party sent in the first round
    pub commitment_msg: MsgId,
    /// ID of the message that party sent in the second round
    pub decommitment_msg: MsgId,
}

#[cfg(test)]
mod tests {
    use alloc::{vec, vec::Vec};

    use rand::Rng;
    use round_based::simulation::Simulation;
    use sha2::{Digest, Sha256};

    use super::{protocol_of_random_generation, Msg};

    #[tokio::test]
    async fn simulation_async() {
        let mut rng = rand_dev::DevRng::new();

        let n: u16 = 5;

        let mut simulation = Simulation::<Msg>::new();
        let mut party_output = vec![];

        for i in 0..n {
            let party = simulation.add_party();
            let output = protocol_of_random_generation(party, i, n, rng.fork());
            party_output.push(output);
        }

        let output = futures::future::try_join_all(party_output).await.unwrap();

        // Assert that all parties outputed the same randomness
        for i in 1..n {
            assert_eq!(output[0], output[usize::from(i)]);
        }

        std::println!("Output randomness: {}", hex::encode(output[0]));
    }

    #[test]
    fn simulation_sync() {
        let mut rng = rand_dev::DevRng::new();

        let simulation = round_based::simulation::SimulationSync::from_async_fn(5, |i, party| {
            protocol_of_random_generation(party, i, 5, rng.fork())
        });

        let outputs = simulation
            .run()
            .unwrap()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for output_i in &outputs {
            assert_eq!(*output_i, outputs[0]);
        }
    }

    // Emulate the protocol using the state machine interface
    #[test]
    fn state_machine() {
        use super::{CommitMsg, DecommitMsg, Msg};
        use round_based::{
            state_machine::{ProceedResult, StateMachine},
            Incoming, Outgoing,
        };

        let mut rng = rand_dev::DevRng::new();

        let party1_rng: [u8; 32] = rng.gen();
        let party1_com = Sha256::digest(party1_rng);

        let party2_rng: [u8; 32] = rng.gen();
        let party2_com = Sha256::digest(party2_rng);

        // Start the protocol
        let mut party0 = round_based::state_machine::wrap_protocol(|party| async {
            protocol_of_random_generation(party, 0, 3, rng).await
        });

        // Round 1

        // Party sends its commitment
        let ProceedResult::SendMsg(Outgoing {
            msg: Msg::CommitMsg(party0_com),
            ..
        }) = party0.proceed()
        else {
            panic!("unexpected response")
        };

        // Round 2

        // Party needs messages sent by other parties in round 1
        let ProceedResult::NeedsOneMoreMessage = party0.proceed() else {
            panic!("unexpected response")
        };
        // Provide message from party 1
        party0
            .received_msg(Incoming {
                id: 0,
                sender: 1,
                msg_type: round_based::MessageType::Broadcast,
                msg: Msg::CommitMsg(CommitMsg {
                    commitment: party1_com,
                }),
            })
            .unwrap();
        let ProceedResult::NeedsOneMoreMessage = party0.proceed() else {
            panic!("unexpected response")
        };
        // Provide message from party 2
        party0
            .received_msg(Incoming {
                id: 1,
                sender: 2,
                msg_type: round_based::MessageType::Broadcast,
                msg: Msg::CommitMsg(CommitMsg {
                    commitment: party2_com,
                }),
            })
            .unwrap();

        // Party sends message in round 2
        let ProceedResult::SendMsg(Outgoing {
            msg: Msg::DecommitMsg(party0_rng),
            ..
        }) = party0.proceed()
        else {
            panic!("unexpected response")
        };

        {
            // Check that commitment matches the revealed randomness
            let expected = Sha256::digest(party0_rng.randomness);
            assert_eq!(party0_com.commitment, expected);
        }

        // Final round

        // Party needs messages sent by other parties in round 2
        let ProceedResult::NeedsOneMoreMessage = party0.proceed() else {
            panic!("unexpected response")
        };
        // Provide message from party 1
        party0
            .received_msg(Incoming {
                id: 3,
                sender: 1,
                msg_type: round_based::MessageType::Broadcast,
                msg: Msg::DecommitMsg(DecommitMsg {
                    randomness: party1_rng,
                }),
            })
            .unwrap();
        let ProceedResult::NeedsOneMoreMessage = party0.proceed() else {
            panic!("unexpected response")
        };
        // Provide message from party 2
        party0
            .received_msg(Incoming {
                id: 3,
                sender: 2,
                msg_type: round_based::MessageType::Broadcast,
                msg: Msg::DecommitMsg(DecommitMsg {
                    randomness: party2_rng,
                }),
            })
            .unwrap();
        // Obtain the protocol result
        let ProceedResult::Output(Ok(output_rng)) = party0.proceed() else {
            panic!("unexpected response")
        };

        let output_expected = party0_rng
            .randomness
            .iter()
            .zip(&party1_rng)
            .zip(&party2_rng)
            .map(|((a, b), c)| a ^ b ^ c)
            .collect::<alloc::vec::Vec<_>>();
        assert_eq!(output_rng, output_expected.as_slice());
    }
}
