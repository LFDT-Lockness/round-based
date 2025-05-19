#![allow(dead_code)] // TODO: remove

use alloc::vec::Vec;

use digest::Digest;

use crate::{
    round::{RoundInput, RoundStore},
    Incoming,
};

use super::error;

const TAG: &[u8] = b"dfns.round_based.echo_broadcast";

pub enum State<D: Digest, S: RoundStore> {
    /// A round from principal protocol is ongoing
    PrincipalRound(PrincipalRound<D, S>),
    /// Principal round is completed, we need to send an echo message
    SendEchoMsg(digest::Output<D>, EchoRound<D, S::Output>),
    /// Principal round is completed, echo round is ongoing
    EchoRound(EchoRound<D, S::Output>),

    /// Principal and echo rounds are finished, reliability check has passed
    Output(S::Output),

    /// Indicates that the round has previously already returned the output.
    /// Calling any methods when state is finished results into an error.
    Finished,

    /// Indicates that the state is temporarily moved. Calling methods when state
    /// is gone results into an error and indicates a bug.
    Gone,
}

struct PrincipalRound<D: Digest, S: RoundStore> {
    my_msg: Option<S::Msg>,
    received_msgs: Vec<Option<S::Msg>>,
    store: S,
    received_echoes: RoundInput<digest::Output<D>>,
}

struct EchoRound<D: Digest, O> {
    store_output: O,
    received_echoes: RoundInput<digest::Output<D>>,
    expected_hash: digest::Output<D>,
}

impl<D: Digest, S: RoundStore> State<D, S> {
    pub fn init(store: S, i: u16, n: u16) -> Self {
        State::PrincipalRound(PrincipalRound {
            my_msg: None,
            received_msgs: core::iter::repeat_with(|| None).take(n.into()).collect(),
            store,
            received_echoes: crate::round::broadcast(i, n),
        })
    }

    /// Takes the state by value, replaces `self` with `State::Gone`.
    ///
    /// `self` must be overwritten. Not overwriting a `State::Gone` is a bug
    fn take(&mut self) -> Result<Self, error::Reason> {
        match core::mem::replace(self, State::Gone) {
            State::Gone => Err(error::Reason::StateGone),
            state => Ok(state),
        }
    }

    /// Indicates that [`take_output`] method will return `Some(_)`
    pub fn ready_to_output(&self) -> bool {
        matches!(self, State::Output(_))
    }

    /// Retrieves the round output if reliability check has passed
    pub fn take_output(&mut self) -> Option<S::Output> {
        match self.take().ok()? {
            State::Output(out) => {
                *self = State::Finished;
                Some(out)
            }
            state => {
                *self = state;
                None
            }
        }
    }
}
impl<D, S> State<D, S>
where
    D: Digest,
    S: RoundStore,
    S::Msg: udigest::Digestable + Clone,
{
    pub fn received_principal_msg(
        &mut self,
        incoming: Incoming<S::Msg>,
    ) -> Result<(), error::Error<S::Error>> {
        let state = self.take()?;
        match state.received_principal_msg_inner(incoming) {
            Ok(next_state) => {
                *self = next_state;
                Ok(())
            }
            Err(err) => {
                *self = State::Finished;
                Err(err)
            }
        }
    }
    fn received_principal_msg_inner(
        self,
        incoming: Incoming<S::Msg>,
    ) -> Result<Self, error::Error<S::Error>> {
        match self {
            State::PrincipalRound(mut principal_round) => {
                principal_round
                    .store
                    .add_message(incoming.clone())
                    .map_err(error::Error::Principal)?;
                let n = principal_round.received_msgs.len();
                let slot = principal_round
                    .received_msgs
                    .get_mut(usize::from(incoming.sender))
                    .ok_or(error::Reason::UnknownSender {
                        i: incoming.sender,
                        n,
                    })?;
                if slot.is_some() {
                    return Err(error::Reason::StoreReceivedTwoMsgsFromSameParty.into());
                }
                *slot = Some(incoming.msg);

                principal_round.advance_if_possible().map_err(Into::into)
            }
            State::SendEchoMsg(..) | State::EchoRound(_) | State::Output(_) => {
                Err(error::Reason::ReceivedPrincipalMsgWhenRoundOver.into())
            }
            State::Finished => Err(error::Reason::StateFinished.into()),
            State::Gone => Err(error::Reason::StateGone.into()),
        }
    }

    pub fn received_echo_msg(
        &mut self,
        incoming: Incoming<digest::Output<D>>,
    ) -> Result<(), error::EchoError> {
        let state = self.take()?;
        match state.received_echo_msg_inner(incoming) {
            Ok(next_state) => {
                *self = next_state;
                Ok(())
            }
            Err(err) => {
                *self = State::Finished;
                Err(err)
            }
        }
    }
    fn received_echo_msg_inner(
        self,
        incoming: Incoming<digest::Output<D>>,
    ) -> Result<Self, error::EchoError> {
        match self {
            State::PrincipalRound(mut round) => {
                round
                    .received_echoes
                    .add_message(incoming)
                    .map_err(error::Reason::HandleEcho)?;
                Ok(State::PrincipalRound(round))
            }
            State::SendEchoMsg(msg, mut round) => {
                round
                    .received_echoes
                    .add_message(incoming)
                    .map_err(error::Reason::HandleEcho)?;
                Ok(State::SendEchoMsg(msg, round))
            }
            State::EchoRound(mut round) => {
                round
                    .received_echoes
                    .add_message(incoming)
                    .map_err(error::Reason::HandleEcho)?;
                round.advance_if_possible().map_err(Into::into)
            }
            State::Output(_output) => Err(error::Reason::ReceivedEchoMsgWhenRoundOver.into()),
            State::Finished => Err(error::Reason::StateFinished.into()),
            State::Gone => Err(error::Reason::StateGone.into()),
        }
    }

    /// Retrieves an echo msg and marks it as sent
    ///
    /// Returns `None` if there's no echo msg to be sent (yet or already)
    pub fn take_echo_msg(&mut self) -> Option<digest::Output<D>> {
        let state = self.take().ok()?;
        let (next_state, msg) = state.take_echo_msg_inner();
        *self = next_state;
        msg
    }
    fn take_echo_msg_inner(self) -> (Self, Option<digest::Output<D>>) {
        match self {
            State::SendEchoMsg(echo_msg, echo_round) => {
                (State::EchoRound(echo_round), Some(echo_msg))
            }
            state => (state, None),
        }
    }
}

impl<D, S> PrincipalRound<D, S>
where
    D: Digest,
    S: RoundStore,
    S::Msg: udigest::Digestable,
{
    fn advance_if_possible(self) -> Result<State<D, S>, error::Reason> {
        if !self.store.wants_more() {
            // Principal round is over, we can start the echo round
            let output = self
                .store
                .output()
                .map_err(|_| error::Reason::PrincipalRoundFinishedButStoreDoesntOutput)?;
            let echo_msg = udigest::hash::<D>(&udigest::inline_struct!(TAG {
                received_msgs: &self.received_msgs,
            }));
            Ok(State::SendEchoMsg(
                echo_msg.clone(),
                EchoRound {
                    store_output: output,
                    received_echoes: self.received_echoes,
                    expected_hash: echo_msg,
                },
            ))
        } else {
            Ok(State::PrincipalRound(self))
        }
    }
}

impl<D, O> EchoRound<D, O>
where
    D: Digest,
{
    fn advance_if_possible<S: RoundStore<Output = O>>(self) -> Result<State<D, S>, error::Reason> {
        if !self.received_echoes.wants_more() {
            // Echo round is over, now handle the received messages
            let echoes = self
                .received_echoes
                .output()
                .map_err(|_| error::Reason::EchoRoundFinishedButStoreDoesntOutput)?;
            if echoes.iter().any(|m| *m != self.expected_hash) {
                // echo check failed, abort!
                Err(error::Reason::MismatchedHash)
            } else {
                Ok(State::Output(self.store_output))
            }
        } else {
            Ok(State::EchoRound(self))
        }
    }
}

pub trait IsFinished {
    fn is_finished(&self) -> bool;
}

impl<D: Digest, S: RoundStore> IsFinished for core::cell::RefCell<State<D, S>> {
    fn is_finished(&self) -> bool {
        self.borrow().ready_to_output()
    }
}
