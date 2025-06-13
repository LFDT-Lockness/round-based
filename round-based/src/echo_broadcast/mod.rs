//! Reliable broadcast for any protocol via echo messages
//!
//! Broadcast message is a message meant to be received by all participants of the protocol.
//!
//! We say that message is reliably broadcasted if, upon reception, it is guaranteed that all
//! honest participants of the protocol has received the same message.
//!
//! One way to achieve the reliable broadcast is by adding an echo round: when we receive
//! messages in a reliable broadcast round, we hash all messages, and we send the hash to all
//! other participants. If party receives a the same hash from everyone else, we can be
//! assured that messages in the round were reliably broadcasted.
//!
//! This module provides a mechanism that automatically add an echo round per each
//! round of the protocol that requires a reliable broadcast.
//!
//! ## Example
//!
//! ```rust
//! # #[derive(round_based::ProtocolMsg, Clone, udigest::Digestable)]
//! # enum KeygenMsg {}
//! # struct KeyShare;
//! # struct Error;
//! # type Result<T> = std::result::Result<T, Error>;
//! # async fn doc() -> Result<()> {
//! // protocol to be executed that **requires** reliable broadcast
//! async fn keygen<M>(mpc: M, i: u16, n: u16) -> Result<KeyShare>
//! where
//!     M: round_based::Mpc<Msg = KeygenMsg>
//! {
//!     // ...
//! # unimplemented!()
//! }
//! // The full message type, which corresponds to keygen msg + echo broadcast msg
//! type Msg = round_based::echo_broadcast::Msg<sha2::Sha256, KeygenMsg>;
//! // establishes network connection(s) to other parties, but
//! // **does not** support reliable broadcast
//! async fn connect() ->
//!     impl futures::Stream<Item = Result<round_based::Incoming<Msg>>>
//!         + futures::Sink<round_based::Outgoing<Msg>, Error = Error>
//!         + Unpin
//! {
//!     // ...
//! # round_based::_docs::fake_delivery()
//! }
//! let delivery = connect().await;
//!
//! # let (i, n) = (1, 3);
//! // constructs an MPC engine as usual
//! let mpc = round_based::mpc::connected(delivery);
//! // wraps an engine to add reliable broadcast support
//! let mpc = round_based::echo_broadcast::wrap(mpc, i, n);
//!
//! // execute the protocol
//! let keyshare = keygen(mpc, i, n).await?;
//! # Ok(()) }
//! ```

use core::marker::PhantomData;

use alloc::collections::btree_map::BTreeMap;
use digest::Digest;

use crate::{
    round::{RoundInfo, RoundStore, RoundStoreExt},
    Mpc, MpcExecution, Outgoing, ProtocolMsg, RoundMsg,
};

mod error;
mod store;

pub use self::error::{CompleteRoundError, EchoError, Error};

/// Message of the protocol with echo broadcast round(s)
pub enum Msg<D: Digest, M> {
    /// Message from echo broadcast sub-protocol
    Echo {
        /// Indicates for which round of main protocol this echo message is transmitted
        ///
        /// Note that this field is controlled by potential malicious party. If it sets it
        /// to the round that doesn't exist, the protocol will likely be aborted with an error
        /// that we received a message from unregistered round, which may appear as implementation
        /// error (i.e. API misuse), but in fact it's a malicious abort.
        round: u16,
        /// Hash of all messages received in `round`
        hash: digest::Output<D>,
    },
    /// Message from the main protocol
    Main(M),
}

/// Sub-messages of [`Msg`]
///
/// Sub-messages implement [`RoundMsg`] trait for [`Msg`]
mod sub_msg {
    pub struct EchoMsg<D: digest::Digest, R> {
        pub hash: digest::Output<D>,
        pub _round: core::marker::PhantomData<R>,
    }
    #[derive(Debug, Clone)]
    pub struct Main<M>(pub M);

    impl<D: digest::Digest, R> Clone for EchoMsg<D, R> {
        fn clone(&self) -> Self {
            Self {
                hash: self.hash.clone(),
                _round: core::marker::PhantomData,
            }
        }
    }
}

impl<D: Digest, M: Clone> Clone for Msg<D, M> {
    fn clone(&self) -> Self {
        match self {
            Self::Echo { round, hash } => Self::Echo {
                round: *round,
                hash: hash.clone(),
            },
            Self::Main(msg) => Self::Main(msg.clone()),
        }
    }
}

impl<D: Digest, M: PartialEq> PartialEq for Msg<D, M> {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::Echo { round, hash } => {
                matches!(other, Self::Echo { round: r2, hash: h2 } if round == r2 && hash == h2)
            }
            Self::Main(msg) => matches!(other, Self::Main(m2) if msg == m2),
        }
    }
}

impl<D: Digest, M: PartialEq> Eq for Msg<D, M> {}

impl<D: Digest, M: core::fmt::Debug> core::fmt::Debug for Msg<D, M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Echo { round, hash } => f
                .debug_struct("Msg::Echo")
                .field("round", round)
                .field("hash", hash)
                .finish(),
            Self::Main(msg) => f.debug_tuple("Msg::Main").field(msg).finish(),
        }
    }
}

impl<D: Digest, M: ProtocolMsg> ProtocolMsg for Msg<D, M> {
    fn round(&self) -> u16 {
        match self {
            Self::Echo { round, .. } => 2 * round + 1,
            Self::Main(m) => 2 * m.round(),
        }
    }
}

impl<D: Digest, M: ProtocolMsg, R> RoundMsg<sub_msg::EchoMsg<D, R>> for Msg<D, M>
where
    M: RoundMsg<R>,
{
    const ROUND: u16 = 2 * M::ROUND + 1;
    fn to_protocol_msg(round_msg: sub_msg::EchoMsg<D, R>) -> Self {
        Self::Echo {
            round: M::ROUND,
            hash: round_msg.hash,
        }
    }
    fn from_protocol_msg(protocol_msg: Self) -> Result<sub_msg::EchoMsg<D, R>, Self> {
        match protocol_msg {
            Self::Echo { round, hash } if round == M::ROUND => Ok(sub_msg::EchoMsg {
                hash,
                _round: PhantomData,
            }),
            _ => Err(protocol_msg),
        }
    }
}

impl<D: Digest, ProtoM, RoundM> RoundMsg<sub_msg::Main<RoundM>> for Msg<D, ProtoM>
where
    ProtoM: ProtocolMsg + RoundMsg<RoundM>,
{
    const ROUND: u16 = 2 * <ProtoM as RoundMsg<RoundM>>::ROUND;
    fn to_protocol_msg(round_msg: sub_msg::Main<RoundM>) -> Self {
        Self::Main(ProtoM::to_protocol_msg(round_msg.0))
    }
    fn from_protocol_msg(protocol_msg: Self) -> Result<sub_msg::Main<RoundM>, Self> {
        if let Self::Main(msg) = protocol_msg {
            ProtoM::from_protocol_msg(msg)
                .map(sub_msg::Main)
                .map_err(|m| Self::Main(m))
        } else {
            Err(protocol_msg)
        }
    }
}

/// Wraps an [`Mpc`] engine and provides echo broadcast capabilities
pub fn wrap<D, M, MainMsg>(party: M, i: u16, n: u16) -> WithEchoBroadcast<D, M, MainMsg>
where
    D: Digest,
    M: Mpc<Msg = Msg<D, MainMsg>>,
    MainMsg: udigest::Digestable,
{
    WithEchoBroadcast {
        party,
        i,
        n,
        sent_reliable_msgs: Default::default(),
        _ph: PhantomData,
    }
}

/// [`Mpc`] engine with echo-broadcast capabilities
pub struct WithEchoBroadcast<D: Digest, M, Msg> {
    party: M,
    i: u16,
    n: u16,
    sent_reliable_msgs: BTreeMap<u16, Option<Msg>>,
    _ph: PhantomData<D>,
}

impl<D: Digest, M, Msg> WithEchoBroadcast<D, M, Msg> {
    fn map_party<P>(self, f: impl FnOnce(M) -> P) -> WithEchoBroadcast<D, P, Msg> {
        let party = f(self.party);
        WithEchoBroadcast {
            party,
            i: self.i,
            n: self.n,
            sent_reliable_msgs: self.sent_reliable_msgs,
            _ph: PhantomData,
        }
    }
}

impl<D, M, MainMsg> Mpc for WithEchoBroadcast<D, M, MainMsg>
where
    D: Digest + 'static,
    M: Mpc<Msg = Msg<D, MainMsg>>,
    MainMsg: ProtocolMsg + udigest::Digestable + Clone + 'static,
{
    type Msg = MainMsg;

    type Exec = WithEchoBroadcast<D, M::Exec, MainMsg>;

    type SendErr = error::Error<M::SendErr>;

    fn add_round<R>(&mut self, round: R) -> <Self::Exec as MpcExecution>::Round<R>
    where
        R: RoundStore,
        Self::Msg: RoundMsg<R::Msg>,
    {
        let reliable_broadcast_required = round
            .read_prop::<crate::round::props::RequiresReliableBroadcast>()
            .map(|x| x.0);
        if reliable_broadcast_required == Some(true) {
            let (main_round, echo_round) = store::new::<D, MainMsg, _>(self.i, self.n, round);
            let main_round = self.party.add_round(store::WithMainMsg(main_round));
            let echo_round = self.party.add_round(store::WithEchoError::from(echo_round));

            self.sent_reliable_msgs.insert(Self::Msg::ROUND, None);

            Round(Inner::WithReliabilityCheck {
                main_round,
                echo_round,
            })
        } else {
            let round = self
                .party
                .add_round(store::WithError(store::WithMainMsg(round)));
            Round(Inner::Unmodified(round))
        }
    }

    fn finish_setup(self) -> Self::Exec {
        self.map_party(|p| p.finish_setup())
    }
}

impl<D, M, MainMsg> WithEchoBroadcast<D, M, MainMsg>
where
    D: Digest,
    MainMsg: ProtocolMsg + Clone,
{
    fn on_send(&mut self, outgoing: &mut Outgoing<MainMsg>) -> Result<(), error::EchoError> {
        if let Some(slot) = self.sent_reliable_msgs.get_mut(&outgoing.msg.round()) {
            if !outgoing.recipient.is_reliable_broadcast() {
                // it's reliable broadcast round, but message is not reliable broadcast
                return Err(error::Reason::SentNonReliableMsgInReliableRound {
                    dest: outgoing.recipient,
                    round: outgoing.msg.round(),
                }
                .into());
            }
            // Message delivery layer doesn't need to know that protocol wants this message to be
            // reliably broadcasted - echo broadcast takes care of it
            outgoing.recipient = crate::MessageDestination::AllParties { reliable: false };
            if slot.is_some() {
                return Err(error::Reason::SendTwice.into());
            }
            *slot = Some(outgoing.msg.clone())
        } else if outgoing.recipient.is_reliable_broadcast() {
            // it's not a reliable broadcast round, but message is a reliable broadcast
            return Err(error::Reason::SentReliableMsgInNonReliableRound {
                round: outgoing.msg.round(),
            }
            .into());
        }

        Ok(())
    }
}

impl<D, M, MainMsg> MpcExecution for WithEchoBroadcast<D, M, MainMsg>
where
    D: Digest + 'static,
    M: MpcExecution<Msg = Msg<D, MainMsg>>,
    MainMsg: ProtocolMsg + udigest::Digestable + Clone + 'static,
{
    type Round<R: RoundInfo> = Round<M, D, MainMsg, R>;
    type Msg = MainMsg;
    type CompleteRoundErr<E> =
        error::CompleteRoundError<M::CompleteRoundErr<error::Error<E>>, M::SendErr>;
    type SendErr = error::Error<M::SendErr>;
    type SendMany = WithEchoBroadcast<D, M::SendMany, MainMsg>;

    async fn complete<R>(
        &mut self,
        round: Self::Round<R>,
    ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
    where
        R: RoundInfo,
        Self::Msg: RoundMsg<R::Msg>,
    {
        match round.0 {
            Inner::Unmodified(round) => {
                // regular round that doesn't need reliable broadcast
                let output = self
                    .party
                    .complete(round)
                    .await
                    .map_err(error::CompleteRoundError::CompleteRound)?;
                Ok(output)
            }
            Inner::WithReliabilityCheck {
                main_round,
                echo_round,
            } => {
                // receive all messages in the main round
                let main_output = self
                    .party
                    .complete(main_round)
                    .await
                    .map_err(error::CompleteRoundError::CompleteRound)?;
                // retrieve a msg that we sent in this round
                let sent_msg =
                    if let Some(Some(msg)) = self.sent_reliable_msgs.remove(&Self::Msg::ROUND) {
                        let msg: R::Msg = Self::Msg::from_protocol_msg(msg)
                            .map_err(|_| error::Reason::SentMsgFromProto)?;
                        Some(msg)
                    } else {
                        None
                    };
                // calculate a hash and send it to all other parties
                let (main_output, hash) = main_output.with_my_msg(sent_msg)?;
                self.party
                    .send_to_all(Msg::Echo {
                        round: Self::Msg::ROUND,
                        hash,
                    })
                    .await
                    .map_err(error::CompleteRoundError::Send)?;
                // receive echoes from other parties
                let echoes = self
                    .party
                    .complete(echo_round)
                    .await
                    .map_err(error::CompleteRoundError::CompleteRound)?;
                // check that everyone sent the same hash
                let main_output = main_output.with_echo_output(echoes)?;

                Ok(main_output)
            }
        }
    }

    async fn send(&mut self, mut outgoing: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.on_send(&mut outgoing)?;

        self.party
            .send(outgoing.map(Msg::Main))
            .await
            .map_err(error::Error::Main)
    }

    fn send_many(self) -> Self::SendMany {
        self.map_party(|p| p.send_many())
    }

    async fn yield_now(&self) {
        self.party.yield_now().await
    }
}

/// Round registration witness returned by [`WithEchoBroadcast::add_round()`]
pub struct Round<M, D, ProtoMsg, R>(Inner<M, D, ProtoMsg, R>)
where
    M: MpcExecution,
    D: Digest + 'static,
    ProtoMsg: 'static,
    R: RoundInfo;

enum Inner<M, D, ProtoMsg, R>
where
    M: MpcExecution,
    D: Digest + 'static,
    ProtoMsg: 'static,
    R: RoundInfo,
{
    /// Round that we do not modify (round that doesn't require reliable broadcast)
    Unmodified(M::Round<store::WithError<store::WithMainMsg<R>>>),
    WithReliabilityCheck {
        main_round: M::Round<store::WithMainMsg<store::MainRound<D, ProtoMsg, R>>>,
        echo_round: M::Round<store::WithEchoError<store::EchoRound<D, R>, R::Error>>,
    },
}

impl<D, M, MainMsg> crate::mpc::SendMany for WithEchoBroadcast<D, M, MainMsg>
where
    D: Digest + 'static,
    M: crate::mpc::SendMany<Msg = Msg<D, MainMsg>>,
    MainMsg: ProtocolMsg + udigest::Digestable + Clone + 'static,
{
    type Exec = WithEchoBroadcast<D, M::Exec, MainMsg>;
    type Msg = MainMsg;
    type SendErr = error::Error<M::SendErr>;

    async fn send(&mut self, mut outgoing: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.on_send(&mut outgoing)?;
        self.party
            .send(outgoing.map(Msg::Main))
            .await
            .map_err(error::Error::Main)
    }

    async fn flush(self) -> Result<Self::Exec, Self::SendErr> {
        let party = self.party.flush().await.map_err(error::Error::Main)?;
        Ok(WithEchoBroadcast {
            party,
            i: self.i,
            n: self.n,
            sent_reliable_msgs: self.sent_reliable_msgs,
            _ph: PhantomData,
        })
    }
}
