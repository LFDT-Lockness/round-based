//! Routes incoming MPC messages between rounds
//!
//! [`RoundsRouter`] is an essential building block of MPC protocol, it processes incoming messages, groups
//! them by rounds, and provides convenient API for retrieving received messages at certain round.
//!
//! ## Example
//!
//! ```rust
//! use round_based::{Mpc, MpcParty, ProtocolMsg, Delivery, PartyIndex};
//! use round_based::rounds_router::{RoundsRouter, simple_store::{RoundInput, RoundMsgs}};
//!
//! #[derive(ProtocolMsg)]
//! pub enum Msg {
//!     Round1(Msg1),
//!     Round2(Msg2),
//! }
//!
//! pub struct Msg1 { /* ... */ }
//! pub struct Msg2 { /* ... */ }
//!
//! pub async fn some_mpc_protocol<M>(party: M, i: PartyIndex, n: u16) -> Result<Output, Error>
//! where
//!     M: Mpc<ProtocolMsg = Msg>,
//! {
//!     let MpcParty{ delivery, .. } = party.into_party();
//!
//!     let (incomings, _outgoings) = delivery.split();
//!
//!     // Build `Rounds`
//!     let mut rounds = RoundsRouter::builder();
//!     let round1 = rounds.add_round(RoundInput::<Msg1>::broadcast(i, n));
//!     let round2 = rounds.add_round(RoundInput::<Msg2>::p2p(i, n));
//!     let mut rounds = rounds.listen(incomings);
//!
//!     // Receive messages from round 1
//!     let msgs: RoundMsgs<Msg1> = rounds.complete(round1).await?;
//!
//!     // ... process received messages
//!
//!     // Receive messages from round 2
//!     let msgs = rounds.complete(round2).await?;
//!
//!     // ...
//!     # todo!()
//! }
//! # type Output = ();
//! # type Error = Box<dyn std::error::Error>;
//! ```

use alloc::{boxed::Box, collections::BTreeMap};
use core::{any::Any, mem};

use phantom_type::PhantomType;
use tracing::{error, trace_span, warn};

use crate::{round::RoundStore, Incoming, ProtocolMsg, RoundMsg};

/// Routes received messages between protocol rounds
pub struct RoundsRouter<M> {
    rounds: BTreeMap<u16, Option<Box<dyn ProcessRoundMessage<Msg = M>>>>,
}

impl<M> RoundsRouter<M>
where
    M: ProtocolMsg + 'static,
{
    pub fn new() -> Self {
        Self {
            rounds: Default::default(),
        }
    }

    /// Registers new round
    ///
    /// ## Panics
    /// Panics if round `R` was already registered
    pub fn add_round<R>(&mut self, message_store: R) -> Round<R>
    where
        R: RoundStore,
        M: RoundMsg<R::Msg>,
    {
        let overridden_round = self.rounds.insert(
            M::ROUND,
            Some(Box::new(ProcessRoundMessageImpl::new(message_store))),
        );
        if overridden_round.is_some() {
            panic!("round {} is overridden", M::ROUND);
        }
        Round {
            _ph: PhantomType::new(),
        }
    }

    pub fn received_msg(&mut self, incoming: Incoming<M>) -> Result<(), errors::UnregisteredRound> {
        let msg_round_n = incoming.msg.round();
        let span = trace_span!(
            "Round::received_msg",
            round = %msg_round_n,
            sender = %incoming.sender,
            ty = ?incoming.msg_type
        );
        let _guard = span.enter();

        let message_round = match self.rounds.get_mut(&msg_round_n) {
            Some(Some(round)) => round,
            Some(None) => {
                warn!("got message for the round that was already completed, ignoring it");
                return Ok(());
            }
            None => {
                return Err(errors::UnregisteredRound {
                    n: msg_round_n,
                    witness_provided: false,
                })
            }
        };
        if message_round.needs_more_messages().no() {
            warn!("received message for the round that was already completed, ignoring it");
            return Ok(());
        }
        message_round.process_message(incoming);
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub fn complete_round<R>(
        &mut self,
        round: Round<R>,
    ) -> Result<Result<R::Output, errors::CompleteRoundError<R::Error>>, Round<R>>
    where
        R: RoundStore,
        M: RoundMsg<R::Msg>,
    {
        let message_round = match self.rounds.get_mut(&M::ROUND) {
            Some(Some(round)) => round,
            Some(None) => {
                return Ok(Err(
                    errors::Bug::RoundGoneButWitnessExists { n: M::ROUND }.into()
                ));
            }
            None => {
                return Ok(Err(errors::UnregisteredRound {
                    n: M::ROUND,
                    witness_provided: true,
                }
                .into()))
            }
        };
        if message_round.needs_more_messages().yes() {
            return Err(round);
        }
        Ok(Self::retrieve_round_output::<R>(message_round))
    }

    fn retrieve_round_output<R>(
        round: &mut Box<dyn ProcessRoundMessage<Msg = M>>,
    ) -> Result<R::Output, errors::CompleteRoundError<R::Error>>
    where
        R: RoundStore,
        M: RoundMsg<R::Msg>,
    {
        match round.take_output() {
            Ok(Ok(any)) => Ok(*any
                .downcast::<R::Output>()
                .or(Err(errors::Bug::MismatchedOutputType))?),
            Ok(Err(any)) => Err(*any
                .downcast::<errors::CompleteRoundError<R::Error>>()
                .or(Err(errors::Bug::MismatchedErrorType))?),
            Err(err) => Err(errors::Bug::TakeRoundResult(err).into()),
        }
    }
}

/// A witness that round has been registered in the router
///
/// Can be used later to claim messages received in this round
pub struct Round<S> {
    _ph: PhantomType<S>,
}

impl<S> core::fmt::Debug for Round<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Round").finish_non_exhaustive()
    }
}

trait ProcessRoundMessage {
    type Msg;

    /// Processes round message
    ///
    /// Before calling this method you must ensure that `.needs_more_messages()` returns `Yes`,
    /// otherwise calling this method is unexpected.
    fn process_message(&mut self, msg: Incoming<Self::Msg>);

    /// Indicated whether the store needs more messages
    ///
    /// If it returns `Yes`, then you need to collect more messages to complete round. If it's `No`
    /// then you need to take the round output by calling `.take_output()`.
    fn needs_more_messages(&self) -> NeedsMoreMessages;

    /// Tries to obtain round output
    ///
    /// Can be called once `process_message()` returned `NeedMoreMessages::No`.
    ///
    /// Returns:
    /// * `Ok(Ok(any))` — round is successfully completed, `any` needs to be downcasted to `MessageStore::Output`
    /// * `Ok(Err(any))` — round has terminated with an error, `any` needs to be downcasted to `CompleteRoundError<MessageStore::Error>`
    /// * `Err(err)` — couldn't retrieve the output, see [`TakeOutputError`]
    #[allow(clippy::type_complexity)]
    fn take_output(&mut self) -> Result<Result<Box<dyn Any>, Box<dyn Any>>, TakeOutputError>;
}

#[derive(Debug, thiserror::Error)]
enum TakeOutputError {
    #[error("output is already taken")]
    AlreadyTaken,
    #[error("output is not ready yet, more messages are needed")]
    NotReady,
}

enum ProcessRoundMessageImpl<S: RoundStore, M: ProtocolMsg + RoundMsg<S::Msg>> {
    InProgress { store: S, _ph: PhantomType<fn(M)> },
    Completed(Result<S::Output, errors::CompleteRoundError<S::Error>>),
    Gone,
}

impl<S: RoundStore, M: ProtocolMsg + RoundMsg<S::Msg>> ProcessRoundMessageImpl<S, M> {
    pub fn new(store: S) -> Self {
        if store.wants_more() {
            Self::InProgress {
                store,
                _ph: Default::default(),
            }
        } else {
            Self::Completed(
                store
                    .output()
                    .map_err(|_| errors::ImproperRoundStore::StoreDidntOutput.into()),
            )
        }
    }
}

impl<S, M> ProcessRoundMessageImpl<S, M>
where
    S: RoundStore,
    M: ProtocolMsg + RoundMsg<S::Msg>,
{
    fn _process_message(
        store: &mut S,
        msg: Incoming<M>,
    ) -> Result<(), errors::CompleteRoundError<S::Error>> {
        let msg = msg.try_map(M::from_protocol_msg).map_err(|msg| {
            errors::Bug::MessageFromAnotherRound {
                actual_number: msg.round(),
                expected_round: M::ROUND,
            }
        })?;

        store
            .add_message(msg)
            .map_err(errors::CompleteRoundError::ProcessMsg)?;
        Ok(())
    }
}

impl<S, M> ProcessRoundMessage for ProcessRoundMessageImpl<S, M>
where
    S: RoundStore,
    M: ProtocolMsg + RoundMsg<S::Msg>,
{
    type Msg = M;

    fn process_message(&mut self, msg: Incoming<Self::Msg>) {
        let store = match self {
            Self::InProgress { store, .. } => store,
            _ => {
                return;
            }
        };

        match Self::_process_message(store, msg) {
            Ok(()) => {
                if store.wants_more() {
                    return;
                }

                let store = match mem::replace(self, Self::Gone) {
                    Self::InProgress { store, .. } => store,
                    _ => {
                        *self = Self::Completed(Err(errors::Bug::IncoherentState {
                            expected: "InProgress",
                            justification:
                                "we checked at beginning of the function that `state` is InProgress",
                        }.into()));
                        return;
                    }
                };

                match store.output() {
                    Ok(output) => *self = Self::Completed(Ok(output)),
                    Err(_err) => {
                        *self = Self::Completed(Err(
                            errors::ImproperRoundStore::StoreDidntOutput.into()
                        ))
                    }
                }
            }
            Err(err) => {
                *self = Self::Completed(Err(err));
            }
        }
    }

    fn needs_more_messages(&self) -> NeedsMoreMessages {
        match self {
            Self::InProgress { .. } => NeedsMoreMessages::Yes,
            _ => NeedsMoreMessages::No,
        }
    }

    fn take_output(&mut self) -> Result<Result<Box<dyn Any>, Box<dyn Any>>, TakeOutputError> {
        match self {
            Self::InProgress { .. } => return Err(TakeOutputError::NotReady),
            Self::Gone => return Err(TakeOutputError::AlreadyTaken),
            _ => (),
        }
        match mem::replace(self, Self::Gone) {
            Self::Completed(Ok(output)) => Ok(Ok(Box::new(output))),
            Self::Completed(Err(err)) => Ok(Err(Box::new(err))),
            _ => unreachable!("it's checked to be completed"),
        }
    }
}

enum NeedsMoreMessages {
    Yes,
    No,
}

#[allow(dead_code)]
impl NeedsMoreMessages {
    pub fn yes(&self) -> bool {
        matches!(self, Self::Yes)
    }
    pub fn no(&self) -> bool {
        matches!(self, Self::No)
    }
}

/// When something goes wrong
pub mod errors {
    use super::TakeOutputError;

    #[derive(Debug, thiserror::Error)]
    #[error("received a message for unregistered round")]
    pub(in crate::mpc) struct UnregisteredRound {
        pub n: u16,
        pub(super) witness_provided: bool,
    }

    /// Error returned when processing incoming messages at certain round
    ///
    /// May indicate malicious behavior (e.g. adversary sent a message that aborts protocol execution)
    /// or some misconfiguration of the protocol network (e.g. received a message from the round that
    /// was not registered via [`Mpc::add_round`](crate::Mpc::add_round)).
    #[derive(Debug, thiserror::Error)]
    pub enum CompleteRoundError<ProcessErr> {
        /// [`RoundStore`](crate::round::RoundStore) returned an error
        ///
        /// Refer to this rounds store documentation to understand why it could fail
        #[error(transparent)]
        ProcessMsg(ProcessErr),

        /// Router error
        ///
        /// Indicates that for some reason router was not able to process a message. This can be the case of:
        /// - Router API misuse \
        ///   E.g. when received a message from the round that was not registered in the router
        /// - Improper [`RoundStore`](crate::round::RoundStore) implementation \
        ///   Indicates that round store is not properly implemented and contains a flaw. \
        ///   For instance, this error is returned when round store indicates that it doesn't need
        ///   any more messages ([`RoundStore::wants_more`](crate::round::RoundStore::wants_more)
        ///   returns `false`), but then it didn't output anything ([`RoundStore::output`](crate::round::RoundStore::output)
        ///   returns `Err(_)`)
        /// - Bug in the router
        ///
        /// This error is always related to some implementation flaw or bug: either in the code that uses
        /// the router, or in the round store implementation, or in the router itself. When implementation
        /// is correct, this error never appears. Thus, it should not be possible for the adversary to "make
        /// this error happen."
        Router(RouterError),
    }

    /// Router error
    ///
    /// Refer to [`CompleteRound::Router`] docs
    #[derive(Debug, thiserror::Error)]
    #[error(transparent)]
    pub struct RouterError(Reason);

    #[derive(Debug, thiserror::Error)]
    pub(super) enum Reason {
        /// Router API has been misused
        ///
        /// For instance, this error is returned when protocol implementation does not register
        /// certain round of the protocol, but then a message from this round is received. In
        /// this case, router doesn't have anywhere to route the message to, so an [`ApiMisuse`]
        /// error is returned.
        #[error("api misuse")]
        ApiMisuse(#[source] ApiMisuse),
        /// Improper [`RoundStore`](crate::round::RoundStore) implementation
        ///
        /// For instance, this error is returned when round store indicates that it doesn't need
        /// any more messages ([`RoundStore::wants_more`](crate::round::RoundStore::wants_more)
        /// returns `false`), but then it didn't output anything ([`RoundStore::output`](crate::round::RoundStore::output)
        /// returns `Err(_)`)
        #[error("improper round store")]
        ImproperRoundStore(#[source] ImproperRoundStore),
        /// Indicates that there's a bug in the router implementation
        #[error("bug (please, open an issue)")]
        Bug(#[source] Bug),
    }

    #[derive(Debug, thiserror::Error)]
    pub(super) enum ApiMisuse {
        #[error(transparent)]
        UnregisteredRound(#[from] UnregisteredRound),
    }

    #[derive(Debug, thiserror::Error)]
    pub(super) enum ImproperRoundStore {
        /// Store indicated that it received enough messages but didn't output
        ///
        /// I.e. [`store.wants_more()`] returned `false`, but `store.output()` returned `Err(_)`.
        #[error("store didn't output")]
        StoreDidntOutput,
    }

    #[derive(Debug, thiserror::Error)]
    pub(super) enum Bug {
        #[error("round is gone, but witness exists")]
        RoundGoneButWitnessExists { n: u16 },
        #[error(
            "message originates from another round: we process messages from round \
            {expected_round}, got message from round {actual_number}"
        )]
        MessageFromAnotherRound {
            expected_round: u16,
            actual_number: u16,
        },
        #[error("state is incoherent, it's expected to be {expected}: {justification}")]
        IncoherentState {
            expected: &'static str,
            justification: &'static str,
        },
        #[error("take round result")]
        TakeRoundResult(#[source] TakeOutputError),
        #[error("mismatched output type")]
        MismatchedOutputType,
        #[error("mismatched error type")]
        MismatchedErrorType,
    }

    macro_rules! impl_round_complete_from {
        ($(|$err:ident: $err_ty:ty| $err_fn:expr),+$(,)?) => {$(
            impl<E> From<$err_ty> for CompleteRoundError<E> {
                fn from($err: $err_ty) -> Self {
                    $err_fn
                }
            }
        )+};
    }

    impl_round_complete_from! {
        |err: ApiMisuse| CompleteRoundError::Router(RouterError(Reason::ApiMisuse(err))),
        |err: ImproperRoundStore| CompleteRoundError::Router(RouterError(Reason::ImproperRoundStore(err))),
        |err: Bug| CompleteRoundError::Router(RouterError(Reason::Bug(err))),
        |err: UnregisteredRound| ApiMisuse::UnregisteredRound(err).into(),
    }
}

#[cfg(test)]
mod tests {
    struct Store;

    #[derive(crate::ProtocolMsg)]
    #[protocol_msg(root = crate)]
    enum FakeProtocolMsg {
        R1(Msg1),
    }
    struct Msg1;

    impl super::RoundStore for Store {
        type Msg = Msg1;
        type Output = ();
        type Error = core::convert::Infallible;

        fn add_message(&mut self, _msg: crate::Incoming<Self::Msg>) -> Result<(), Self::Error> {
            Ok(())
        }
        fn wants_more(&self) -> bool {
            false
        }
        fn output(self) -> Result<Self::Output, Self> {
            Ok(())
        }
    }

    #[test]
    fn complete_round_that_expects_no_messages() {
        let mut rounds = super::RoundsRouter::<FakeProtocolMsg>::new();
        let round1 = rounds.add_round(Store);

        rounds.complete_round(round1).unwrap().unwrap();
    }
}
