use alloc::vec::Vec;
use core::marker::PhantomData;
use digest::Digest;

use crate::{
    round::{RoundInfo, RoundInput, RoundMsgs, RoundStore},
    Incoming, RoundMsg,
};

use super::{error, sub_msg};

const TAG: &[u8] = b"dfns.round_based.echo_broadcast";

pub fn new<D: Digest, ProtoMsg, S: RoundStore>(
    i: u16,
    n: u16,
    main_round: S,
) -> (MainRound<D, ProtoMsg, S>, EchoRound<D, S>) {
    let params = Params { i, n };
    let state = match main_round.output() {
        Ok(output) => MainRoundState::Output { output },
        Err(store) => MainRoundState::Ongoing { store },
    };
    let main_round = MainRound {
        params,
        state,
        received_msgs: core::iter::repeat_with(|| None).take(n.into()).collect(),
        _ph: PhantomData,
    };
    let echo_round = EchoRound {
        echo_round: RoundInput::broadcast(i, n),
        _round: PhantomData,
    };
    (main_round, echo_round)
}

enum MainRoundState<S: RoundInfo> {
    Ongoing { store: S },
    Output { output: S::Output },
    Finished,
    Gone,
}

#[derive(Clone, Copy, Debug)]
struct Params {
    i: u16,
    n: u16,
}

pub struct MainRound<D: Digest, ProtoMsg, S: RoundInfo> {
    params: Params,
    state: MainRoundState<S>,
    received_msgs: Vec<Option<ProtoMsg>>,
    _ph: PhantomData<(D, ProtoMsg)>,
}

impl<D: Digest + 'static, ProtoMsg: 'static, S: RoundInfo> RoundInfo for MainRound<D, ProtoMsg, S> {
    type Msg = S::Msg;
    /// When a main round is finished, we output a builder that can be used to
    /// calculate a hash of messages received by all parties in the reliable
    /// broadcast round. Then this hash needs to be re-sent to all participants.
    ///
    /// Only if we receive the same hash from all parties in [`EchoRound`], only
    /// then we can obtain a main round output.
    type Output = NeedsOwnMsg<D, ProtoMsg, S>;
    type Error = error::Error<S::Error>;
}
impl<D: Digest, ProtoMsg, S: RoundStore> RoundStore for MainRound<D, ProtoMsg, S>
where
    ProtoMsg: RoundMsg<S::Msg> + Clone + 'static,
    D: 'static,
{
    fn add_message(&mut self, mut incoming: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        let wants_more = match &mut self.state {
            MainRoundState::Ongoing { store, .. } => {
                // We pretend that msg was reliably broadcasted even though the reliability check is
                // not yet enforced, however, we do not expose the output of the round unless
                // reliability check has passed.
                incoming.msg_type = crate::MessageType::Broadcast { reliable: true };

                // Note: round msg doesn't implement `Clone`, but ProtoMsg does, so we
                // use a trick to create a clone of incoming msg
                let (incoming1, incoming2) = clone_incoming_round_msg::<ProtoMsg, _>(incoming)
                    .ok_or(error::Reason::RoundMsgClone)?;
                store.add_message(incoming1).map_err(error::Error::Main)?;
                let n = self.received_msgs.len();
                let slot = self
                    .received_msgs
                    .get_mut(usize::from(incoming2.sender))
                    .ok_or(error::Reason::UnknownSender {
                        i: incoming2.sender,
                        n,
                    })?;
                if slot.is_some() {
                    return Err(error::Reason::StoreReceivedTwoMsgsFromSameParty.into());
                }
                *slot = Some(ProtoMsg::to_protocol_msg(incoming2.msg));
                store.wants_more()
            }
            MainRoundState::Gone => return Err(error::Reason::StateGone.into()),
            MainRoundState::Output { .. } | MainRoundState::Finished => {
                return Err(error::Reason::ReceivedMainMsgWhenRoundOver.into())
            }
        };

        if !wants_more {
            let store = core::mem::replace(&mut self.state, MainRoundState::Gone);
            let MainRoundState::Ongoing { store } = store else {
                return Err(error::Reason::UnexpectedMainRoundState.into());
            };
            let Ok(output) = store.output() else {
                self.state = MainRoundState::Finished;
                return Err(error::Reason::MainRoundFinishedButStoreDoesntOutput.into());
            };
            self.state = MainRoundState::Output { output };
        }

        Ok(())
    }

    fn wants_more(&self) -> bool {
        match &self.state {
            MainRoundState::Ongoing { .. } => {
                // Note that on each `add_message` we check if `store.wants_more()`,
                // and if it doesn't we change the state to MainRoundState::Output
                true
            }
            _ => false,
        }
    }

    fn output(self) -> Result<Self::Output, Self> {
        match self.state {
            MainRoundState::Output { output } => Ok(NeedsOwnMsg {
                params: self.params,
                main_round_output: output,
                received_msgs: self.received_msgs,
                _hash: PhantomData,
            }),
            state => Err(Self {
                params: self.params,
                state,
                received_msgs: self.received_msgs,
                _ph: PhantomData,
            }),
        }
    }
}

/// Duplicates a round msg
///
/// This function doesn't require that round msg implements `Clone`, instead it only requires
/// that protocol msg is cloneable. It works by converting round msg into protocol msg, creating
/// two clones, and converting them back to round msg.
///
/// We need this function in places where we know that protocol msg is cloneable, but we can't
/// prove to the compiler that round msg is cloneable as well.
///
/// Function returns `None` only if [`RoundMsg`] implementation is not correct.
fn clone_round_msg<M, R>(round_msg: R) -> Option<(R, R)>
where
    M: RoundMsg<R> + Clone,
{
    let proto_msg = M::to_protocol_msg(round_msg);

    let round_msg1 = M::from_protocol_msg(proto_msg.clone()).ok()?;
    let round_msg2 = M::from_protocol_msg(proto_msg).ok()?;

    Some((round_msg1, round_msg2))
}

/// Similar to [`clone_round_msg`] but accepts [`Incoming<Msg>`](Incoming)
fn clone_incoming_round_msg<M, R>(
    incoming_round_msg: Incoming<R>,
) -> Option<(Incoming<R>, Incoming<R>)>
where
    M: RoundMsg<R> + Clone,
{
    let (msg1, msg2) = clone_round_msg::<M, _>(incoming_round_msg.msg)?;

    let incoming = |msg| Incoming {
        id: incoming_round_msg.id,
        sender: incoming_round_msg.sender,
        msg_type: incoming_round_msg.msg_type,
        msg,
    };
    Some((incoming(msg1), incoming(msg2)))
}

/// An output of [`MainRound`] which needs an own message sent by local party
/// in this round. Once provided in [`NeedsOwnMsg::with_own_msg`], it outputs
/// a hash of messages received by all parties in this round (that needs to be
/// re-sent to all participants), and [`WithReliabilityCheck`] that takes
/// messages received in echo round and outputs main round result only if reliability
/// check passes.
pub struct NeedsOwnMsg<D: Digest, ProtoMsg, S: RoundInfo> {
    params: Params,
    main_round_output: S::Output,
    received_msgs: Vec<Option<ProtoMsg>>,
    _hash: PhantomData<D>,
}

pub struct ReliabilityCheck<D: Digest, S: RoundInfo> {
    expected_hash: digest::Output<D>,
    main_round_output: S::Output,
}

impl<D, ProtoMsg, S> NeedsOwnMsg<D, ProtoMsg, S>
where
    D: Digest,
    S: RoundInfo,
    ProtoMsg: RoundMsg<S::Msg> + udigest::Digestable,
{
    pub fn with_my_msg(
        mut self,
        msg: Option<S::Msg>,
    ) -> Result<(ReliabilityCheck<D, S>, digest::Output<D>), error::EchoError> {
        let n = self.received_msgs.len();
        let msg = msg.map(ProtoMsg::to_protocol_msg);
        *self
            .received_msgs
            .get_mut(usize::from(self.params.i))
            .ok_or(error::Reason::OwnIndexOutOfBounds {
                i: self.params.i,
                n,
            })? = msg;

        let hash = udigest::hash::<D>(&udigest::inline_struct!(TAG {
            msgs: &self.received_msgs,
            round: ProtoMsg::ROUND,
            n: self.params.n,
        }));
        let with_reliability_check = ReliabilityCheck {
            expected_hash: hash.clone(),
            main_round_output: self.main_round_output,
        };
        Ok((with_reliability_check, hash))
    }
}

impl<D, S> ReliabilityCheck<D, S>
where
    D: Digest,
    S: RoundInfo,
{
    pub fn with_echo_output(
        self,
        echo_output: EchoRoundOutput<D, S>,
    ) -> Result<S::Output, error::EchoError> {
        if echo_output
            .received_echoes
            .iter()
            .any(|h| *h != self.expected_hash)
        {
            return Err(error::Reason::MismatchedHash.into());
        }

        Ok(self.main_round_output)
    }
}

pub(super) struct EchoRound<D: Digest, S: RoundInfo> {
    echo_round: RoundInput<digest::Output<D>>,
    _round: PhantomData<S>,
}

pub struct EchoRoundOutput<D: Digest, S> {
    received_echoes: RoundMsgs<digest::Output<D>>,
    _round: PhantomData<S>,
}

impl<D, S> RoundInfo for EchoRound<D, S>
where
    D: Digest + 'static,
    S: RoundInfo,
{
    type Msg = sub_msg::EchoMsg<D, S::Msg>;
    type Output = EchoRoundOutput<D, S>;
    type Error = error::EchoError;
}
impl<D, S> RoundStore for EchoRound<D, S>
where
    D: Digest + 'static,
    S: RoundStore,
{
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        self.echo_round
            .add_message(msg.map(|m| m.hash))
            .map_err(error::Reason::HandleEcho)?;
        Ok(())
    }

    fn wants_more(&self) -> bool {
        self.echo_round.wants_more()
    }

    fn output(self) -> Result<Self::Output, Self> {
        self.echo_round
            .output()
            .map(|received_echoes| EchoRoundOutput {
                received_echoes,
                _round: PhantomData,
            })
            .map_err(|echo_round| Self {
                echo_round,
                _round: PhantomData,
            })
    }
}

/// Wraps a round store `S` and changes its msg type to `sub_msg::Main<S::Msg>`
pub struct WithMainMsg<S>(pub S);

impl<S: RoundInfo> RoundInfo for WithMainMsg<S> {
    type Msg = sub_msg::Main<S::Msg>;
    type Output = S::Output;
    type Error = S::Error;
}

impl<S: RoundStore> RoundStore for WithMainMsg<S> {
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        self.0.add_message(msg.map(|m| m.0))
    }
    fn wants_more(&self) -> bool {
        self.0.wants_more()
    }
    fn output(self) -> Result<Self::Output, Self> {
        self.0.output().map_err(Self)
    }
}

/// Wraps a round store `S` and changes its error to `Error<S::Error>`
pub struct WithError<S>(pub S);

impl<S: RoundInfo> RoundInfo for WithError<S> {
    type Msg = S::Msg;
    type Output = S::Output;
    type Error = error::Error<S::Error>;
}

impl<S: RoundStore> RoundStore for WithError<S> {
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        self.0.add_message(msg).map_err(error::Error::Main)
    }
    fn wants_more(&self) -> bool {
        self.0.wants_more()
    }
    fn output(self) -> Result<Self::Output, Self> {
        self.0.output().map_err(Self)
    }
}

/// Wraps a round store `S` with `Error = error::EchoError` and changes it to `Error = error::Error<E>`
pub struct WithEchoError<S, E> {
    pub store: S,
    _ph: PhantomData<E>,
}

impl<S, E> From<S> for WithEchoError<S, E> {
    fn from(store: S) -> Self {
        Self {
            store,
            _ph: PhantomData,
        }
    }
}

impl<S, E> RoundInfo for WithEchoError<S, E>
where
    S: RoundInfo<Error = error::EchoError>,
    E: core::error::Error + 'static,
{
    type Msg = S::Msg;
    type Output = S::Output;
    type Error = error::Error<E>;
}

impl<S, E> RoundStore for WithEchoError<S, E>
where
    S: RoundStore<Error = error::EchoError>,
    E: core::error::Error + 'static,
{
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        self.store.add_message(msg).map_err(error::Error::Echo)
    }
    fn wants_more(&self) -> bool {
        self.store.wants_more()
    }
    fn output(self) -> Result<Self::Output, Self> {
        self.store.output().map_err(|store| Self {
            store,
            _ph: PhantomData,
        })
    }
}
