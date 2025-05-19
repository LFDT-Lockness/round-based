use alloc::vec::Vec;
use core::marker::PhantomData;
use digest::Digest;

use crate::{
    round::{RoundInput, RoundMsgs, RoundStore},
    Incoming,
};

use super::{error, EchoMsg};

const TAG: &[u8] = b"dfns.round_based.echo_broadcast";

enum MainRoundState<S: RoundStore> {
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

pub struct MainRound<D: Digest, S: RoundStore> {
    params: Params,
    state: MainRoundState<S>,
    received_msgs: Vec<Option<S::Msg>>,
    _digest: PhantomData<D>,
}

impl<D: Digest, S: RoundStore> RoundStore for MainRound<D, S>
where
    S::Msg: Clone,
    D: 'static,
{
    type Msg = S::Msg;
    /// When a main round is finished, we output a builder that can be used to
    /// calculate a hash of messages received by all parties in the reliable
    /// broadcast round. Then this hash needs to be re-sent to all participants.
    ///
    /// Only if we receive the same hash from all parties in [`EchoRound`], only
    /// then we can obtain a main round output.
    type Output = NeedsOwnMsg<D, S>;
    type Error = error::Error<S::Error>;

    fn add_message(&mut self, incoming: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        let wants_more = match &mut self.state {
            MainRoundState::Ongoing { store, .. } => {
                store
                    .add_message(incoming.clone())
                    .map_err(error::Error::Principal)?;
                let n = self.received_msgs.len();
                let slot = self
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
                store.wants_more()
            }
            MainRoundState::Gone => return Err(error::Reason::StateGone.into()),
            MainRoundState::Output { .. } | MainRoundState::Finished => {
                return Err(error::Reason::ReceivedPrincipalMsgWhenRoundOver.into())
            }
        };

        if !wants_more {
            let store = core::mem::replace(&mut self.state, MainRoundState::Gone);
            let MainRoundState::Ongoing { store } = store else {
                return Err(error::Reason::UnexpectedMainRoundState.into());
            };
            let Ok(output) = store.output() else {
                self.state = MainRoundState::Finished;
                return Err(error::Reason::PrincipalRoundFinishedButStoreDoesntOutput.into());
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
            state => {
                return Err(Self {
                    params: self.params,
                    state,
                    received_msgs: self.received_msgs,
                    _digest: PhantomData,
                })
            }
        }
    }
}

/// An output of [`MainRound`] which needs an own message sent by local party
/// in this round. Once provided in [`NeedsOwnMsg::with_own_msg`], it outputs
/// a hash of messages received by all parties in this round (that needs to be
/// re-sent to all participants), and [`WithReliabilityCheck`] that takes
/// messages received in echo round and outputs main round result only if reliability
/// check passes.
pub struct NeedsOwnMsg<D: Digest, S: RoundStore> {
    params: Params,
    main_round_output: S::Output,
    received_msgs: Vec<Option<S::Msg>>,
    _hash: PhantomData<D>,
}

pub struct WithReliabilityCheck<D: Digest, S: RoundStore> {
    expected_hash: digest::Output<D>,
    main_round_output: S::Output,
}

impl<D, S> NeedsOwnMsg<D, S>
where
    D: Digest,
    S: RoundStore,
    S::Msg: udigest::Digestable,
{
    pub fn with_my_msg(
        mut self,
        msg: S::Msg,
    ) -> Result<(WithReliabilityCheck<D, S>, digest::Output<D>), error::EchoError> {
        let n = self.received_msgs.len();
        *self
            .received_msgs
            .get_mut(usize::from(self.params.i))
            .ok_or(error::Reason::OwnIndexOutOfBounds {
                i: self.params.i,
                n,
            })? = Some(msg);

        let hash = udigest::hash::<D>(&udigest::inline_struct!(TAG {
            msgs: &self.received_msgs,
            n: self.params.n,
        }));
        let with_reliability_check = WithReliabilityCheck {
            expected_hash: hash.clone(),
            main_round_output: self.main_round_output,
        };
        Ok((with_reliability_check, hash))
    }
}

impl<D, S> WithReliabilityCheck<D, S>
where
    D: Digest,
    S: RoundStore,
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

pub struct EchoRound<D: Digest, S: RoundStore> {
    echo_round: RoundInput<digest::Output<D>>,
    _round: PhantomData<S>,
}

pub struct EchoRoundOutput<D: Digest, S> {
    received_echoes: RoundMsgs<digest::Output<D>>,
    _round: PhantomData<S>,
}

impl<D, S> RoundStore for EchoRound<D, S>
where
    D: Digest + 'static,
    S: RoundStore,
{
    type Msg = EchoMsg<D, S::Msg>;
    type Output = EchoRoundOutput<D, S>;
    type Error = error::EchoError;

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
