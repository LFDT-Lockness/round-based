use core::marker::PhantomData;

use alloc::vec::Vec;
use digest::Digest;

use crate::{
    round::RoundStore, Incoming, Mpc, MpcExecution, MsgId, Outgoing, PartyIndex, ProtocolMsg,
    RoundMsg,
};

mod error;
mod store;

// TODO: remove
// mod state;

pub enum Msg<D: Digest, M> {
    /// Message from echo broadcast sub-protocol
    Echo {
        /// Indicates for which round of main protocol this echo message is transmitted
        round: u16,
        /// Hash of all messages received in `round`
        hash: digest::Output<D>,
    },
    /// Message from the main protocol
    Main(M),
}

struct EchoMsg<D: Digest, R> {
    hash: digest::Output<D>,
    _round: PhantomData<R>,
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

impl<D: Digest, R> Clone for EchoMsg<D, R> {
    fn clone(&self) -> Self {
        Self {
            hash: self.hash.clone(),
            _round: PhantomData,
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

impl<D: Digest, M: ProtocolMsg, R> RoundMsg<EchoMsg<D, R>> for Msg<D, M>
where
    M: RoundMsg<R>,
{
    const ROUND: u16 = 2 * M::ROUND + 1;
    fn to_protocol_msg(round_msg: EchoMsg<D, R>) -> Self {
        Self::Echo {
            round: M::ROUND,
            hash: round_msg.hash,
        }
    }
    fn from_protocol_msg(protocol_msg: Self) -> Result<EchoMsg<D, R>, Self> {
        match protocol_msg {
            Self::Echo { round, hash } if round == M::ROUND => Ok(EchoMsg {
                hash,
                _round: PhantomData,
            }),
            _ => Err(protocol_msg),
        }
    }
}

struct Principal<M>(M);

impl<D: Digest, ProtoM, RoundM> RoundMsg<Principal<RoundM>> for Msg<D, ProtoM>
where
    ProtoM: ProtocolMsg + RoundMsg<RoundM>,
{
    const ROUND: u16 = 2 * <ProtoM as RoundMsg<RoundM>>::ROUND;
    fn to_protocol_msg(round_msg: Principal<RoundM>) -> Self {
        Self::Main(ProtoM::to_protocol_msg(round_msg.0))
    }
    fn from_protocol_msg(protocol_msg: Self) -> Result<Principal<RoundM>, Self> {
        if let Self::Main(msg) = protocol_msg {
            ProtoM::from_protocol_msg(msg)
                .map(Principal)
                .map_err(|m| Self::Main(m))
        } else {
            Err(protocol_msg)
        }
    }
}

pub fn wrap<D, M, PrincipalMsg>(party: M, i: u16) -> WithReliableBroadcast<D, M>
where
    D: Digest,
    M: Mpc<Msg = Msg<D, PrincipalMsg>>,
    PrincipalMsg: udigest::Digestable,
{
    todo!()
}

pub struct WithReliableBroadcast<D: Digest, M> {
    party: M,
    i: u16,
    n: u16,
    _ph: PhantomData<D>,
}

impl<D, M, PrincipalMsg> Mpc for WithReliableBroadcast<D, M>
where
    D: Digest,
    M: Mpc<Msg = Msg<D, PrincipalMsg>>,
    PrincipalMsg: Clone,
{
    type Msg = PrincipalMsg;

    type Exec = WithReliableBroadcast<D, M::Exec>;

    type SendErr = M::SendErr;

    fn add_round<R>(&mut self, round: R) -> <Self::Exec as MpcExecution>::Round<R>
    where
        R: RoundStore,
        Self::Msg: RoundMsg<R::Msg>,
    {
        // let reliable_broadcast_required = round
        //     .read_prop::<crate::round::props::RequiresReliableBroadcast>()
        //     .map(|x| x.0);
        // if reliable_broadcast_required == Some(true) {
        //     let round_state = state::State::<D, _>::init(round, self.i, self.n);
        //     let round_state = core::cell::RefCell::new(round_state);
        //     let round_state = alloc::rc::Rc::new(round_state);

        //     let store = store::RoundWithEcho(round_state.clone());

        //     todo!()
        // }
        // Round(self.party.add_round(Principal(round)))
        todo!()
    }

    fn finish(self) -> Self::Exec {
        // WithReliableBroadcast {
        //     party: self.party.finish(),
        //     i: self.i,
        //     _ph: PhantomData,
        // }
        todo!()
    }
}

impl<D, M, PrincipalMsg> MpcExecution for WithReliableBroadcast<D, M>
where
    D: Digest,
    M: MpcExecution<Msg = Msg<D, PrincipalMsg>>,
    PrincipalMsg: Clone,
{
    type Round<R> = Round<M, D, R>;
    type Msg = PrincipalMsg;
    type CompleteRoundErr<E> = M::CompleteRoundErr<E>;
    type SendErr = M::SendErr;
    type SendMany = SendMany<D, M::SendMany>;

    async fn complete<R>(
        &mut self,
        round: Self::Round<R>,
    ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
    where
        R: RoundStore,
        Self::Msg: RoundMsg<R::Msg>,
    {
        match round.0 {
            Inner::Unmodified(round) => self.party.complete(round).await,
            Inner::WithReliabilityCheck {
                main_round,
                echo_round,
            } => {
                todo!()
            }
        }
    }

    async fn receive_and_process_one_message(
        &mut self,
    ) -> Result<(), Self::CompleteRoundErr<core::convert::Infallible>> {
        self.party.receive_and_process_one_message().await
    }

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.party.send(msg.map(Msg::Main)).await
    }

    fn send_many(self) -> Self::SendMany {
        SendMany {
            sender: self.party.send_many(),
            i: self.i,
            _ph: PhantomData,
        }
    }

    async fn yield_now(&self) {
        self.party.yield_now().await
    }
}

pub struct Round<M: MpcExecution, D: Digest, R: RoundStore>(Inner<M, D, R>);

enum Inner<M: MpcExecution, D: Digest, R: RoundStore> {
    /// Round that we do not modify (round that doesn't require reliable broadcast)
    Unmodified(M::Round<Principal<R>>),
    WithReliabilityCheck {
        main_round: M::Round<store::MainRound<D, R>>,
        echo_round: M::Round<store::EchoRound<D, R>>,
    },
}

impl<R: RoundStore> RoundStore for Principal<R> {
    type Msg = Principal<R::Msg>;
    type Output = R::Output;
    type Error = R::Error;

    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        self.0.add_message(msg.map(|m| m.0))
    }
    fn wants_more(&self) -> bool {
        self.0.wants_more()
    }
    fn output(self) -> Result<Self::Output, Self> {
        self.0.output().map_err(|s| Principal(s))
    }
}

pub struct SendMany<D: Digest, M: crate::mpc::SendMany> {
    sender: M,
    i: u16,
    _ph: PhantomData<D>,
}

impl<D, M, PrincipalMsg> crate::mpc::SendMany for SendMany<D, M>
where
    D: Digest,
    M: crate::mpc::SendMany<Msg = Msg<D, PrincipalMsg>>,
    PrincipalMsg: Clone,
{
    type Exec = WithReliableBroadcast<D, M::Exec>;
    type Msg = PrincipalMsg;
    type SendErr = M::SendErr;

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.sender.send(msg.map(Msg::Main)).await
    }

    async fn flush(self) -> Result<Self::Exec, Self::SendErr> {
        let party = self.sender.flush().await?;
        // Ok(WithReliableBroadcast {
        //     party,
        //     i: self.i,
        //     _ph: PhantomData,
        // })
        todo!()
    }
}

type SharedRoundState<D, M> =
    alloc::rc::Rc<core::cell::RefCell<Result<RoundState<D, M>, RoundStateError>>>;

struct RoundState<D: Digest, M> {
    my_msg: Option<M>,
    received_msgs: Vec<Option<M>>,
    received_echoes: Vec<Option<digest::Output<D>>>,
}

#[derive(Debug, thiserror::Error)]
enum RoundStateError {
    /// Party sent two messages in one round
    ///
    /// `msgs_ids` are ids of conflicting messages
    #[error("party {sender} tried to overwrite message")]
    AttemptToOverwriteReceivedMsg {
        /// IDs of conflicting messages
        msgs_ids: [MsgId; 2],
        /// Index of party who sent two messages in one round
        sender: PartyIndex,
    },
}
