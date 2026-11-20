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
//!
//! ## Buffered rounds
//! Some protocols need to process messages of a round manually, one-by-one, rather than receive
//! all of them at once when the round is complete. Such round can be registered via
//! [`RoundsRouterBuilder::add_buffered_round`], then its messages are retrieved by calling
//! [`RoundsRouter::next_msg_in`]:
//!
//! ```rust
//! use round_based::{Mpc, MpcParty, ProtocolMessage, Delivery};
//! use round_based::rounds_router::{RoundsRouter, round_buf::RoundBufBounded};
//!
//! #[derive(ProtocolMessage)]
//! pub enum Msg {
//!     Round1(Msg1),
//! }
//!
//! pub struct Msg1 { /* ... */ }
//!
//! pub async fn some_mpc_protocol<M>(party: M) -> Result<Output, Error>
//! where
//!     M: Mpc<ProtocolMessage = Msg>,
//! {
//!     let MpcParty{ delivery, .. } = party.into_party();
//!
//!     let (incomings, _outgoings) = delivery.split();
//!
//!     // Build `Rounds`
//!     let mut rounds = RoundsRouter::builder();
//!     let mut round1 = rounds.add_buffered_round(RoundBufBounded::<Msg1>::new(10));
//!     let mut rounds = rounds.listen(incomings);
//!
//!     loop {
//!         // Receive next message from round 1
//!         let msg = rounds.next_msg_in(&mut round1).await?;
//!
//!         // ... process received message, break the loop when enough messages are received
//!         # break
//!     }
//!
//!     // Once `round1` is dropped, the round is closed: router ignores its messages
//!     drop(round1);
//!
//!     // ...
//!     # todo!()
//! }
//! # type Output = ();
//! # type Error = Box<dyn std::error::Error>;
//! ```

use alloc::{boxed::Box, collections::BTreeMap, sync::Arc};
use core::{
    any::Any,
    convert::Infallible,
    mem,
    sync::atomic::{AtomicBool, Ordering},
};

use futures_util::{Stream, StreamExt};
use phantom_type::PhantomType;
use tracing::{error, trace, warn};

use crate::Incoming;

use self::round_buf::RoundBuf;

#[doc(inline)]
pub use self::errors::{CompleteRoundError, ReceiveMsgError};
pub use self::store::*;

pub mod round_buf;
pub mod simple_store;
mod store;

/// Routes received messages between protocol rounds
///
/// See [module level](self) documentation to learn more about it.
pub struct RoundsRouter<M, S = ()> {
    incomings: S,
    rounds: BTreeMap<u16, Option<Box<dyn ProcessRoundMessage<Msg = M> + Send>>>,
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
    E: core::error::Error,
{
    /// Completes specified round
    ///
    /// Waits until all messages at specified round are received. Returns received
    /// messages if round is successfully completed, or error otherwise.
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(n = <M as RoundMessage<R::Msg>>::ROUND)
    )]
    pub async fn complete<R>(
        &mut self,
        round: Round<R>,
    ) -> Result<R::Output, CompleteRoundError<R::Error, E>>
    where
        R: MessagesStore,
        M: RoundMessage<R::Msg>,
    {
        match self.complete_inner(round).await {
            Ok(output) => {
                trace!("round successfully completed");
                Ok(output)
            }
            Err(err) => {
                error!(%err, "round terminated with error");
                Err(err)
            }
        }
    }

    async fn complete_inner<R>(
        &mut self,
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
            self.route_message(incoming)?;

            if pending_round == message_round_n {
                if let Some(output) = self.retrieve_round_output_if_its_completed::<R>() {
                    return output;
                }
            }
        }
    }

    /// Routes incoming message to the round it belongs to
    fn route_message(&mut self, incoming: Incoming<M>) -> Result<(), errors::RoundsMisuse> {
        let message_round_n = incoming.msg.round();

        let message_round_slot = self
            .rounds
            .get_mut(&message_round_n)
            .ok_or(errors::RoundsMisuse::UnregisteredRound { n: message_round_n })?;
        if matches!(message_round_slot, Some(round) if round.buf_round_is_closed()) {
            // buffered round is closed, messages that it buffered are not needed anymore
            *message_round_slot = None;
        }
        let message_round = match message_round_slot {
            Some(round) => round,
            None => {
                warn!(
                    n = message_round_n,
                    "got message for the round that was already completed, ignoring it"
                );
                return Ok(());
            }
        };
        if message_round.needs_more_messages().no() {
            warn!(
                n = message_round_n,
                "received message for the round that was already completed, ignoring it"
            );
            return Ok(());
        }
        message_round.process_message(incoming);
        Ok(())
    }

    /// Receives next message in specified buffered round
    ///
    /// Returns a message buffered by the [`RoundBuf`], if there's any. Otherwise, waits until
    /// a message of this round is received. Messages of other rounds that are received in the
    /// meantime are routed to their rounds.
    ///
    /// Returns error if the message cannot be received. In particular, if `RoundBuf` failed to
    /// buffer a message, this method returns the messages that are already buffered, then it
    /// returns [`ReceiveMsgError::Buffer`]. After the error is returned, the round is completed:
    /// its messages are ignored.
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(n = <M as RoundMessage<B::Msg>>::ROUND),
    )]
    pub async fn next_msg_in<B>(
        &mut self,
        _round: &mut BufferedRound<B>,
    ) -> Result<Incoming<B::Msg>, ReceiveMsgError<B::Error, E>>
    where
        B: RoundBuf,
        M: RoundMessage<B::Msg> + 'static,
    {
        match self.next_msg_in_inner::<B>().await {
            Ok(msg) => {
                trace!("buffered message received");
                Ok(msg)
            }
            Err(err) => {
                error!(%err, "buffered round terminated with error");
                Err(err)
            }
        }
    }

    async fn next_msg_in_inner<B>(
        &mut self,
    ) -> Result<Incoming<B::Msg>, ReceiveMsgError<B::Error, E>>
    where
        B: RoundBuf,
        M: RoundMessage<B::Msg> + 'static,
    {
        loop {
            if let Some(msg) = self.retrieve_buffered_msg::<B>() {
                return msg;
            }

            let incoming = match self.incomings.next().await {
                Some(Ok(msg)) => msg,
                Some(Err(err)) => return Err(errors::IoError::Io(err).into()),
                None => return Err(errors::IoError::UnexpectedEof.into()),
            };
            self.route_message(incoming)?;
        }
    }

    #[allow(clippy::type_complexity)]
    fn retrieve_buffered_msg<B>(
        &mut self,
    ) -> Option<Result<Incoming<B::Msg>, ReceiveMsgError<B::Error, E>>>
    where
        B: RoundBuf,
        M: RoundMessage<B::Msg> + 'static,
    {
        let round_number = <M as RoundMessage<B::Msg>>::ROUND;
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
        let round = match round
            .as_mut_any()
            .downcast_mut::<BufferedRoundState<B, M>>()
            .ok_or(errors::RoundsMisuse::NotBufferedRound { n: round_number })
        {
            Ok(round) => round,
            Err(err) => return Some(Err(err.into())),
        };

        if let Some(msg) = round.buf.next_message() {
            return Some(Ok(msg));
        }
        // all buffered messages are retrieved, now we can output the error if it occurred
        let err = round.error.take()?;
        *round_slot = None;
        Some(Err(err.map_io_err(|e| match e {})))
    }

    #[allow(clippy::type_complexity)]
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
    rounds: BTreeMap<u16, Option<Box<dyn ProcessRoundMessage<Msg = M> + Send>>>,
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
            rounds: BTreeMap::new(),
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
            Some(Box::new(ProcessRoundMessageImpl::new(message_store))),
        );
        if overridden_round.is_some() {
            panic!("round {} is overridden", M::ROUND);
        }
        Round {
            _ph: PhantomType::new(),
        }
    }

    /// Registers new buffered round
    ///
    /// Messages of buffered round are processed by the protocol manually, one-by-one: they can be
    /// retrieved by calling [`RoundsRouter::next_msg_in`]. Messages that are received before the
    /// protocol asked for them are buffered in `round_buf`.
    ///
    /// Round stays open until returned [`BufferedRound`] is dropped. Once it's dropped, router
    /// ignores all messages of this round.
    ///
    /// ## Panics
    /// Panics if round `B::Msg` was already registered
    pub fn add_buffered_round<B>(&mut self, round_buf: B) -> BufferedRound<B>
    where
        B: RoundBuf + Send,
        B::Error: Send,
        M: RoundMessage<B::Msg>,
    {
        let closed = Arc::new(AtomicBool::new(false));
        let overridden_round = self.rounds.insert(
            M::ROUND,
            Some(Box::new(BufferedRoundState::<B, M> {
                buf: round_buf,
                error: None,
                closed: closed.clone(),
                _ph: PhantomType::new(),
            })),
        );
        if overridden_round.is_some() {
            panic!("round {} is overridden", M::ROUND);
        }
        BufferedRound {
            closed,
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

/// A buffered round of MPC protocol
///
/// `BufferedRound` can be used to retrieve messages received at this round one-by-one by calling
/// [`RoundsRouter::next_msg_in`]. See [module level](self) documentation to see usage.
///
/// When `BufferedRound` is dropped, the round is closed: router ignores all messages of this
/// round that it receives afterwards.
pub struct BufferedRound<B: RoundBuf> {
    closed: Arc<AtomicBool>,
    _ph: PhantomType<B>,
}

impl<B: RoundBuf> Drop for BufferedRound<B> {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
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

    /// Indicates whether the round is a buffered round which handle was dropped
    fn buf_round_is_closed(&self) -> bool {
        false
    }

    /// Returns `self` as `Any`, so it can be downcasted to the concrete type of the round
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

#[derive(Debug, thiserror::Error)]
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

impl<S: MessagesStore, M: ProtocolMessage + RoundMessage<S::Msg>> ProcessRoundMessageImpl<S, M> {
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
                    .map_err(|_| errors::ImproperStoreImpl::StoreDidntOutput.into()),
            )
        }
    }
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
    M: ProtocolMessage + RoundMessage<S::Msg> + 'static,
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

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

struct BufferedRoundState<B: RoundBuf, M> {
    buf: B,
    /// Error occurred when processing a message, after that round accepts no more messages
    error: Option<ReceiveMsgError<B::Error, Infallible>>,
    /// Set to `true` when [`BufferedRound`] is dropped
    closed: Arc<AtomicBool>,
    _ph: PhantomType<fn(M)>,
}

impl<B, M> BufferedRoundState<B, M>
where
    B: RoundBuf,
    M: ProtocolMessage + RoundMessage<B::Msg>,
{
    fn _process_message(
        buf: &mut B,
        msg: Incoming<M>,
    ) -> Result<(), ReceiveMsgError<B::Error, Infallible>> {
        let msg = msg.try_map(M::from_protocol_message).map_err(|msg| {
            errors::Bug::MessageFromAnotherRound {
                actual_number: msg.round(),
                expected_round: M::ROUND,
            }
        })?;

        buf.add_message(msg).map_err(ReceiveMsgError::Buffer)?;
        Ok(())
    }
}

impl<B, M> ProcessRoundMessage for BufferedRoundState<B, M>
where
    B: RoundBuf,
    M: ProtocolMessage + RoundMessage<B::Msg> + 'static,
{
    type Msg = M;

    fn process_message(&mut self, msg: Incoming<Self::Msg>) {
        if self.error.is_some() {
            return;
        }
        if let Err(err) = Self::_process_message(&mut self.buf, msg) {
            self.error = Some(err);
        }
    }

    fn needs_more_messages(&self) -> NeedsMoreMessages {
        if self.error.is_some() {
            NeedsMoreMessages::No
        } else {
            NeedsMoreMessages::Yes
        }
    }

    fn take_output(&mut self) -> Result<Result<Box<dyn Any>, Box<dyn Any>>, TakeOutputError> {
        // buffered round never outputs, its messages are retrieved one-by-one
        Err(TakeOutputError::NotReady)
    }

    fn buf_round_is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
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

    /// Error indicating that `Rounds` failed to complete certain round
    #[derive(Debug, thiserror::Error)]
    pub enum CompleteRoundError<ProcessErr, IoErr> {
        /// [`MessagesStore`](super::MessagesStore) failed to process this message
        #[error("failed to process the message")]
        ProcessMessage(#[source] ProcessErr),
        /// Receiving next message resulted into i/o error
        #[error("receive next message")]
        Io(#[from] IoError<IoErr>),
        /// Some implementation specific error
        ///
        /// Error may be result of improper `MessagesStore` implementation, API misuse, or bug
        /// in `Rounds` implementation
        #[error("implementation error")]
        Other(#[source] OtherError),
    }

    /// Error indicating that `Rounds` failed to receive next message in buffered round
    #[derive(Debug, thiserror::Error)]
    pub enum ReceiveMsgError<BufErr, IoErr> {
        /// [`RoundBuf`](super::round_buf::RoundBuf) failed to buffer a message
        #[error("failed to buffer the message")]
        Buffer(#[source] BufErr),
        /// Receiving next message resulted into i/o error
        #[error("receive next message")]
        Io(#[from] IoError<IoErr>),
        /// Some implementation specific error
        ///
        /// Error may be result of API misuse, or bug in `Rounds` implementation
        #[error("implementation error")]
        Other(#[source] OtherError),
    }

    /// Error indicating that receiving next message resulted into i/o error
    #[derive(Debug, thiserror::Error)]
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
    #[derive(Debug, thiserror::Error)]
    #[error(transparent)]
    pub struct OtherError(OtherReason);

    #[derive(Debug, thiserror::Error)]
    pub(super) enum OtherReason {
        #[error("improper `MessagesStore` implementation")]
        ImproperStoreImpl(#[source] ImproperStoreImpl),
        #[error("`Rounds` API misuse")]
        RoundsMisuse(#[source] RoundsMisuse),
        #[error("bug in `Rounds` (please, open a issue)")]
        Bug(#[source] Bug),
    }

    #[derive(Debug, thiserror::Error)]
    pub(super) enum ImproperStoreImpl {
        /// Store indicated that it received enough messages but didn't output
        ///
        /// I.e. [`store.wants_more()`] returned `false`, but `store.output()` returned `Err(_)`.
        #[error("store didn't output")]
        StoreDidntOutput,
    }

    #[derive(Debug, thiserror::Error)]
    pub(super) enum RoundsMisuse {
        #[error("round is already completed")]
        RoundAlreadyCompleted,
        #[error("round {n} is not registered")]
        UnregisteredRound { n: u16 },
        #[error("round {n} is not registered as buffered round with this type of buffer")]
        NotBufferedRound { n: u16 },
    }

    #[derive(Debug, thiserror::Error)]
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
        /// Maps I/O error
        pub fn map_io_err<E, F>(self, f: F) -> CompleteRoundError<ProcessErr, E>
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

    impl<BufErr, IoErr> ReceiveMsgError<BufErr, IoErr> {
        /// Maps I/O error
        pub fn map_io_err<E, F>(self, f: F) -> ReceiveMsgError<BufErr, E>
        where
            F: FnOnce(IoErr) -> E,
        {
            match self {
                ReceiveMsgError::Io(err) => ReceiveMsgError::Io(err.map_err(f)),
                ReceiveMsgError::Buffer(err) => ReceiveMsgError::Buffer(err),
                ReceiveMsgError::Other(err) => ReceiveMsgError::Other(err),
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
            impl<E1, E2> From<$err> for ReceiveMsgError<E1, E2> {
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

#[cfg(test)]
mod tests {
    struct Store;

    #[derive(crate::ProtocolMessage)]
    #[protocol_message(root = crate)]
    enum FakeProtocolMsg {
        R1(Msg1),
        R2(Msg2),
    }
    struct Msg1;
    #[derive(Debug, PartialEq)]
    struct Msg2(u8);

    type Incomings =
        alloc::vec::Vec<Result<crate::Incoming<FakeProtocolMsg>, core::convert::Infallible>>;

    fn incoming(sender: u16, msg: FakeProtocolMsg) -> crate::Incoming<FakeProtocolMsg> {
        crate::Incoming {
            id: 0,
            sender,
            msg_type: crate::MessageType::Broadcast,
            msg,
        }
    }

    impl super::MessagesStore for Store {
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

    #[tokio::test]
    async fn complete_round_that_expects_no_messages() {
        let incomings = futures::stream::pending::<
            Result<crate::Incoming<FakeProtocolMsg>, core::convert::Infallible>,
        >();

        let mut rounds = super::RoundsRouter::builder();
        let round1 = rounds.add_round(Store);
        let mut rounds = rounds.listen(incomings);

        rounds.complete(round1).await.unwrap();
    }

    #[tokio::test]
    async fn buffered_round_outputs_messages_one_by_one() {
        use super::{round_buf::RoundBufUnbounded, simple_store::RoundInput};

        // messages of round 2 are received before, in the middle, and after messages of round 1
        let incomings: Incomings = alloc::vec![
            Ok(incoming(1, FakeProtocolMsg::R2(Msg2(10)))),
            Ok(incoming(1, FakeProtocolMsg::R1(Msg1))),
            Ok(incoming(2, FakeProtocolMsg::R2(Msg2(20)))),
            Ok(incoming(2, FakeProtocolMsg::R1(Msg1))),
            Ok(incoming(1, FakeProtocolMsg::R2(Msg2(11)))),
        ];

        let mut rounds = super::RoundsRouter::builder();
        let round1 = rounds.add_round(RoundInput::<Msg1>::broadcast(0, 3));
        let mut round2 = rounds.add_buffered_round(RoundBufUnbounded::<Msg2>::new());
        let mut rounds = rounds.listen(futures::stream::iter(incomings));

        rounds.complete(round1).await.unwrap();

        for (sender, msg) in [(1, Msg2(10)), (2, Msg2(20)), (1, Msg2(11))] {
            let received = rounds.next_msg_in(&mut round2).await.unwrap();
            assert_eq!(received.sender, sender);
            assert_eq!(received.msg, msg);
        }

        let err = rounds.next_msg_in(&mut round2).await.unwrap_err();
        assert!(matches!(
            err,
            super::ReceiveMsgError::Io(super::errors::IoError::UnexpectedEof)
        ));
    }

    #[tokio::test]
    async fn buffered_round_outputs_buffered_messages_before_error() {
        use super::{round_buf::RoundBufBounded, simple_store::RoundInput};

        let incomings: Incomings = alloc::vec![
            Ok(incoming(1, FakeProtocolMsg::R2(Msg2(10)))),
            Ok(incoming(2, FakeProtocolMsg::R2(Msg2(20)))),
            // buffer overflows
            Ok(incoming(1, FakeProtocolMsg::R2(Msg2(11)))),
            // ignored as the round is terminated with error
            Ok(incoming(2, FakeProtocolMsg::R2(Msg2(21)))),
            Ok(incoming(1, FakeProtocolMsg::R1(Msg1))),
            Ok(incoming(2, FakeProtocolMsg::R1(Msg1))),
        ];

        let mut rounds = super::RoundsRouter::builder();
        let round1 = rounds.add_round(RoundInput::<Msg1>::broadcast(0, 3));
        let mut round2 = rounds.add_buffered_round(RoundBufBounded::<Msg2>::new(2));
        let mut rounds = rounds.listen(futures::stream::iter(incomings));

        rounds.complete(round1).await.unwrap();

        for msg in [Msg2(10), Msg2(20)] {
            let received = rounds.next_msg_in(&mut round2).await.unwrap();
            assert_eq!(received.msg, msg);
        }
        let err = rounds.next_msg_in(&mut round2).await.unwrap_err();
        assert!(matches!(err, super::ReceiveMsgError::Buffer(_)));

        // once the error is returned, round is completed
        let err = rounds.next_msg_in(&mut round2).await.unwrap_err();
        assert!(matches!(err, super::ReceiveMsgError::Other(_)));
    }

    #[tokio::test]
    async fn dropped_buffered_round_ignores_messages() {
        use super::{round_buf::RoundBufBounded, simple_store::RoundInput};

        // round 2 is closed, so its messages do not overflow the buffer
        let incomings: Incomings = alloc::vec![
            Ok(incoming(1, FakeProtocolMsg::R2(Msg2(10)))),
            Ok(incoming(2, FakeProtocolMsg::R2(Msg2(20)))),
            Ok(incoming(1, FakeProtocolMsg::R1(Msg1))),
            Ok(incoming(2, FakeProtocolMsg::R1(Msg1))),
        ];

        let mut rounds = super::RoundsRouter::builder();
        let round1 = rounds.add_round(RoundInput::<Msg1>::broadcast(0, 3));
        let round2 = rounds.add_buffered_round(RoundBufBounded::<Msg2>::new(0));
        let mut rounds = rounds.listen(futures::stream::iter(incomings));

        drop(round2);
        rounds.complete(round1).await.unwrap();
        let round2 = <FakeProtocolMsg as super::RoundMessage<Msg2>>::ROUND;
        assert!(rounds.rounds[&round2].is_none());
    }
}
