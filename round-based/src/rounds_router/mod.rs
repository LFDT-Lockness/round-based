//! Routes incoming MPC messages between rounds
//!
//! [`RoundsRouter`] is an essential building block of MPC protocol, it processes incoming messages, groups
//! them by rounds, and provides convenient API for retrieving received messages at certain round.
//!
//! ## Example
//!
//! ```rust
//! use round_based::{Mpc, MpcParty, ProtocolMessage, Delivery, PartyIndex};
//! use round_based::rounds_router::{RoundsRouter, simple_store::{RoundInput, RoundMsgs}};
//!
//! #[derive(ProtocolMessage)]
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
//!     M: Mpc<ProtocolMessage = Msg>,
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

use std::any::Any;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Debug;
use std::mem;

use futures_util::{Stream, StreamExt};
use phantom_type::PhantomType;
use thiserror::Error;
use tracing::{debug, error, trace, trace_span, warn, Span};

use crate::Incoming;

#[doc(inline)]
pub use self::errors::CompleteRoundError;
pub use self::store::*;

pub mod simple_store;
mod store;

/// Routes received messages between protocol rounds
///
/// See [module level](self) documentation to learn more about it.
pub struct RoundsRouter<M, S = ()> {
    incomings: S,
    rounds: HashMap<u16, Option<Box<dyn ProcessRoundMessage<Msg = M> + Send>>>,
}

impl<M: ProtocolMessage + 'static> RoundsRouter<M> {
    /// Instantiates [`RoundsRouterBuilder`]
    pub fn builder() -> RoundsRouterBuilder<M> {
        RoundsRouterBuilder::new()
    }
}

impl<M, S, E> RoundsRouter<M, S>
where
    M: ProtocolMessage,
    S: Stream<Item = Result<Incoming<M>, E>> + Unpin,
{
    /// Completes specified round
    ///
    /// Waits until all messages at specified round are received. Returns received
    /// messages if round is successfully completed, or error otherwise.
    #[inline(always)]
    pub async fn complete<R>(
        &mut self,
        round: Round<R>,
    ) -> Result<R::Output, CompleteRoundError<R::Error, E>>
    where
        R: MessagesStore,
        M: RoundMessage<R::Msg>,
    {
        let round_number = <M as RoundMessage<R::Msg>>::ROUND;
        let span = trace_span!("Round", n = round_number);
        debug!(parent: &span, "pending round to complete");

        match self.complete_with_span(&span, round).await {
            Ok(output) => {
                trace!(parent: &span, "round successfully completed");
                Ok(output)
            }
            Err(err) => {
                error!(parent: &span, %err, "round terminated with error");
                Err(err)
            }
        }
    }

    async fn complete_with_span<R>(
        &mut self,
        span: &Span,
        _round: Round<R>,
    ) -> Result<R::Output, CompleteRoundError<R::Error, E>>
    where
        R: MessagesStore,
        M: RoundMessage<R::Msg>,
    {
        let pending_round = <M as RoundMessage<R::Msg>>::ROUND;
        if let Some(output) = self.retrieve_round_output_if_its_completed::<R>() {
            return output;
        }

        loop {
            let incoming = match self.incomings.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(err)) => return Err(errors::IoError::Io(err).into()),
                None => return Err(errors::IoError::UnexpectedEof.into()),
            };
            let message_round_n = incoming.msg.round();

            let message_round = match self.rounds.get_mut(&message_round_n) {
                Some(Some(round)) => round,
                Some(None) => {
                    warn!(
                        parent: span,
                        n = message_round_n,
                        "got message for the round that was already completed, ignoring it"
                    );
                    continue;
                }
                None => {
                    return Err(
                        errors::RoundsMisuse::UnregisteredRound { n: message_round_n }.into(),
                    )
                }
            };
            if message_round.needs_more_messages().no() {
                warn!(
                    parent: span,
                    n = message_round_n,
                    "received message for the round that was already completed, ignoring it"
                );
                continue;
            }
            message_round.process_message(incoming);

            if pending_round == message_round_n {
                if let Some(output) = self.retrieve_round_output_if_its_completed::<R>() {
                    return output;
                }
            }
        }
    }

    fn retrieve_round_output_if_its_completed<R>(
        &mut self,
    ) -> Option<Result<R::Output, CompleteRoundError<R::Error, E>>>
    where
        R: MessagesStore,
        M: RoundMessage<R::Msg>,
    {
        let round_number = <M as RoundMessage<R::Msg>>::ROUND;
        let round_slot = match self
            .rounds
            .get_mut(&round_number)
            .ok_or(errors::RoundsMisuse::UnregisteredRound { n: round_number })
        {
            Ok(slot) => slot,
            Err(err) => return Some(Err(err.into())),
        };
        let round = match round_slot
            .as_mut()
            .ok_or(errors::RoundsMisuse::RoundAlreadyCompleted)
        {
            Ok(round) => round,
            Err(err) => return Some(Err(err.into())),
        };
        if round.needs_more_messages().no() {
            Some(Self::retrieve_round_output::<R>(round_slot))
        } else {
            None
        }
    }

    fn retrieve_round_output<R>(
        slot: &mut Option<Box<dyn ProcessRoundMessage<Msg = M> + Send>>,
    ) -> Result<R::Output, CompleteRoundError<R::Error, E>>
    where
        R: MessagesStore,
        M: RoundMessage<R::Msg>,
    {
        let mut round = slot.take().ok_or(errors::RoundsMisuse::UnregisteredRound {
            n: <M as RoundMessage<R::Msg>>::ROUND,
        })?;
        match round.take_output() {
            Ok(Ok(any)) => Ok(*any
                .downcast::<R::Output>()
                .or(Err(CompleteRoundError::from(
                    errors::Bug::MismatchedOutputType,
                )))?),
            Ok(Err(any)) => Err(any
                .downcast::<CompleteRoundError<R::Error, Infallible>>()
                .or(Err(CompleteRoundError::from(
                    errors::Bug::MismatchedErrorType,
                )))?
                .map_io_err(|e| match e {})),
            Err(err) => Err(errors::Bug::TakeRoundResult(err).into()),
        }
    }
}

/// Builds [`RoundsRouter`]
pub struct RoundsRouterBuilder<M> {
    rounds: HashMap<u16, Option<Box<dyn ProcessRoundMessage<Msg = M> + Send>>>,
}

impl<M> Default for RoundsRouterBuilder<M>
where
    M: ProtocolMessage + 'static,
 {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> RoundsRouterBuilder<M>
where
    M: ProtocolMessage + 'static,
{
    /// Constructs [`RoundsRouterBuilder`]
    ///
    /// Alias to [`RoundsRouter::builder`]
    pub fn new() -> Self {
        Self {
            rounds: HashMap::new(),
        }
    }

    /// Registers new round
    ///
    /// ## Panics
    /// Panics if round `R` was already registered
    pub fn add_round<R>(&mut self, message_store: R) -> Round<R>
    where
        R: MessagesStore + Send + 'static,
        R::Output: Send,
        R::Error: Send,
        M: RoundMessage<R::Msg>,
    {
        let overridden_round = self.rounds.insert(
            M::ROUND,
            Some(Box::new(ProcessRoundMessageImpl::InProgress {
                store: message_store,
                _ph: PhantomType::new(),
            })),
        );
        if overridden_round.is_some() {
            panic!("round {} is overridden", M::ROUND);
        }
        Round {
            _ph: PhantomType::new(),
        }
    }

    /// Builds [`RoundsRouter`]
    ///
    /// Takes a stream of incoming messages which will be routed between registered rounds
    pub fn listen<S, E>(self, incomings: S) -> RoundsRouter<M, S>
    where
        S: Stream<Item = Result<Incoming<M>, E>>,
    {
        RoundsRouter {
            incomings,
            rounds: self.rounds,
        }
    }
}

/// A round of MPC protocol
///
/// `Round` can be used to retrieve messages received at this round by calling [`RoundsRouter::complete`]. See
/// [module level](self) documentation to see usage.
pub struct Round<S: MessagesStore> {
    _ph: PhantomType<S>,
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
    fn take_output(&mut self) -> Result<Result<Box<dyn Any>, Box<dyn Any>>, TakeOutputError>;
}

#[derive(Debug, Error)]
enum TakeOutputError {
    #[error("output is already taken")]
    AlreadyTaken,
    #[error("output is not ready yet, more messages are needed")]
    NotReady,
}

enum ProcessRoundMessageImpl<S: MessagesStore, M: ProtocolMessage + RoundMessage<S::Msg>> {
    InProgress { store: S, _ph: PhantomType<fn(M)> },
    Completed(Result<S::Output, CompleteRoundError<S::Error, Infallible>>),
    Gone,
}

impl<S, M> ProcessRoundMessageImpl<S, M>
where
    S: MessagesStore,
    M: ProtocolMessage + RoundMessage<S::Msg>,
{
    fn _process_message(
        store: &mut S,
        msg: Incoming<M>,
    ) -> Result<(), CompleteRoundError<S::Error, Infallible>> {
        let msg = msg.try_map(M::from_protocol_message).map_err(|msg| {
            errors::Bug::MessageFromAnotherRound {
                actual_number: msg.round(),
                expected_round: M::ROUND,
            }
        })?;

        store
            .add_message(msg)
            .map_err(CompleteRoundError::ProcessMessage)?;
        Ok(())
    }
}

impl<S, M> ProcessRoundMessage for ProcessRoundMessageImpl<S, M>
where
    S: MessagesStore,
    M: ProtocolMessage + RoundMessage<S::Msg>,
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
                        }
                        .into()));
                        return;
                    }
                };

                match store.output() {
                    Ok(output) => *self = Self::Completed(Ok(output)),
                    Err(_err) => {
                        *self =
                            Self::Completed(Err(errors::ImproperStoreImpl::StoreDidntOutput.into()))
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
    use thiserror::Error;

    use super::TakeOutputError;

    /// Error indicating that `Rounds` failed to complete certain round
    #[derive(Debug, Error)]
    pub enum CompleteRoundError<ProcessErr, IoErr> {
        /// [`MessagesStore`](super::MessagesStore) failed to process this message
        #[error("failed to process the message")]
        ProcessMessage(#[source] ProcessErr),
        /// Receiving next message resulted into i/o error
        #[error("receive next message")]
        Io(
            #[source]
            #[from]
            IoError<IoErr>,
        ),
        /// Some implementation specific error
        ///
        /// Error may be result of improper `MessagesStore` implementation, API misuse, or bug
        /// in `Rounds` implementation
        #[error("implementation error")]
        Other(#[source] OtherError),
    }

    /// Error indicating that receiving next message resulted into i/o error
    #[derive(Error, Debug)]
    pub enum IoError<E> {
        /// I/O error
        #[error("i/o error")]
        Io(#[source] E),
        /// Encountered unexpected EOF
        #[error("unexpected eof")]
        UnexpectedEof,
    }

    /// Some implementation specific error
    ///
    /// Error may be result of improper `MessagesStore` implementation, API misuse, or bug
    /// in `Rounds` implementation
    #[derive(Error, Debug)]
    #[error(transparent)]
    pub struct OtherError(OtherReason);

    #[derive(Error, Debug)]
    pub(super) enum OtherReason {
        #[error("improper `MessagesStore` implementation")]
        ImproperStoreImpl(ImproperStoreImpl),
        #[error("`Rounds` API misuse")]
        RoundsMisuse(RoundsMisuse),
        #[error("bug in `Rounds` (please, open a issue)")]
        Bug(Bug),
    }

    #[derive(Debug, Error)]
    pub(super) enum ImproperStoreImpl {
        /// Store indicated that it received enough messages but didn't output
        ///
        /// I.e. [`store.wants_more()`] returned `false`, but `store.output()` returned `Err(_)`.
        #[error("store didn't output")]
        StoreDidntOutput,
    }

    #[derive(Debug, Error)]
    pub(super) enum RoundsMisuse {
        #[error("round is already completed")]
        RoundAlreadyCompleted,
        #[error("round {n} is not registered")]
        UnregisteredRound { n: u16 },
    }

    #[derive(Debug, Error)]
    pub(super) enum Bug {
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
        #[error("mismatched output type")]
        MismatchedOutputType,
        #[error("mismatched error type")]
        MismatchedErrorType,
        #[error("take round result")]
        TakeRoundResult(#[source] TakeOutputError),
    }

    impl<ProcessErr, IoErr> CompleteRoundError<ProcessErr, IoErr> {
        pub(super) fn map_io_err<E, F>(self, f: F) -> CompleteRoundError<ProcessErr, E>
        where
            F: FnOnce(IoErr) -> E,
        {
            match self {
                CompleteRoundError::Io(err) => CompleteRoundError::Io(err.map_err(f)),
                CompleteRoundError::ProcessMessage(err) => CompleteRoundError::ProcessMessage(err),
                CompleteRoundError::Other(err) => CompleteRoundError::Other(err),
            }
        }
    }

    impl<E> IoError<E> {
        pub(super) fn map_err<B, F>(self, f: F) -> IoError<B>
        where
            F: FnOnce(E) -> B,
        {
            match self {
                IoError::Io(e) => IoError::Io(f(e)),
                IoError::UnexpectedEof => IoError::UnexpectedEof,
            }
        }
    }

    macro_rules! impl_from_other_error {
        ($($err:ident),+,) => {$(
            impl<E1, E2> From<$err> for CompleteRoundError<E1, E2> {
                fn from(err: $err) -> Self {
                    Self::Other(OtherError(OtherReason::$err(err)))
                }
            }
        )+};
    }

    impl_from_other_error! {
        ImproperStoreImpl,
        RoundsMisuse,
        Bug,
    }
}
