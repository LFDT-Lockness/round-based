//! Provides [`MpcParty`], default engine for MPC protocol execution that implements [`Mpc`] and [`MpcExecution`] traits

use futures_util::{Sink, SinkExt, Stream, StreamExt};

use crate::{
    round::{RoundInfo, RoundStore},
    Incoming, Outgoing,
};

use super::{Mpc, MpcExecution, ProtocolMsg, RoundMsg};

mod router;
pub mod runtime;

pub use self::router::{errors::RouterError, Round};
#[doc(no_inline)]
pub use self::runtime::AsyncRuntime;

/// MPC engine, carries out the protocol
///
/// Can be constructed via [`MpcParty::connected`] or [`MpcParty::connected_halves`], which wraps
/// a channel of incoming and outgoing messages, and implements additional logic on top of this
/// to facilitate the MPC protocol execution, such as routing incoming messages between round
/// stores.
///
/// Implements [`Mpc`] and [`MpcExecution`].
pub struct MpcParty<M, D, R = runtime::DefaultRuntime, const SETUP_COMPLETE: bool = false> {
    router: router::RoundsRouter<M>,
    io: D,
    runtime: R,
}

impl<M, D, E> MpcParty<M, D>
where
    M: ProtocolMsg + 'static,
    D: Stream<Item = Result<Incoming<M>, E>> + Unpin,
    D: Sink<Outgoing<M>, Error = E> + Unpin,
{
    /// Constructs [`MpcParty`]
    pub fn connected(delivery: D) -> Self {
        Self {
            router: router::RoundsRouter::new(),
            io: delivery,
            runtime: runtime::DefaultRuntime::default(),
        }
    }
}

impl<M, In, Out, E> MpcParty<M, Halves<In, Out>>
where
    M: ProtocolMsg + 'static,
    In: Stream<Item = Result<Incoming<M>, E>> + Unpin,
    Out: Sink<Outgoing<M>, Error = E> + Unpin,
{
    /// Constructs [`MpcParty`]
    pub fn connected_halves(incomings: In, outgoings: Out) -> Self {
        Self::connected(Halves::new(incomings, outgoings))
    }
}

impl<M, D, X> MpcParty<M, D, X> {
    /// Changes which async runtime to use
    pub fn with_runtime<R>(self, runtime: R) -> MpcParty<M, D, R> {
        MpcParty {
            router: self.router,
            io: self.io,
            runtime,
        }
    }
}

impl<M, D, E, AsyncR> Mpc for MpcParty<M, D, AsyncR>
where
    M: ProtocolMsg + 'static,
    D: Stream<Item = Result<Incoming<M>, E>> + Unpin,
    D: Sink<Outgoing<M>, Error = E> + Unpin,
    AsyncR: runtime::AsyncRuntime,
{
    type Msg = M;

    type Exec = MpcParty<M, D, AsyncR, true>;

    type SendErr = E;

    fn add_round<R>(&mut self, round: R) -> <Self::Exec as MpcExecution>::Round<R>
    where
        R: RoundStore,
        Self::Msg: RoundMsg<R::Msg>,
    {
        self.router.add_round(round)
    }

    fn finish(self) -> Self::Exec {
        MpcParty {
            router: self.router,
            io: self.io,
            runtime: self.runtime,
        }
    }
}

impl<M, D, IoErr, AsyncR> MpcExecution for MpcParty<M, D, AsyncR, true>
where
    M: ProtocolMsg + 'static,
    D: Stream<Item = Result<Incoming<M>, IoErr>> + Unpin,
    D: Sink<Outgoing<M>, Error = IoErr> + Unpin,
    AsyncR: runtime::AsyncRuntime,
{
    type Round<R: RoundInfo> = router::Round<R>;
    type Msg = M;
    type CompleteRoundErr<E> = CompleteRoundError<E, IoErr>;
    type SendErr = IoErr;
    type SendMany = SendMany<M, D, AsyncR>;

    async fn complete<R>(
        &mut self,
        mut round: Self::Round<R>,
    ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
    where
        R: RoundInfo,
        Self::Msg: RoundMsg<R::Msg>,
    {
        // Check if round is already completed
        round = match self.router.complete_round(round) {
            Ok(output) => return output.map_err(|e| e.map_io_err(|e| match e {})),
            Err(w) => w,
        };

        // Round is not completed - we need more messages
        loop {
            self.receive_and_process_one_message()
                .await
                .map_err(|e| e.map_process_err(|e| match e {}))?;

            // Check if round was just completed
            round = match self.router.complete_round(round) {
                Ok(output) => return output.map_err(|e| e.map_io_err(|e| match e {})),
                Err(w) => w,
            };
        }
    }

    async fn receive_and_process_one_message(
        &mut self,
    ) -> Result<(), Self::CompleteRoundErr<core::convert::Infallible>> {
        let incoming = self
            .io
            .next()
            .await
            .ok_or(CompleteRoundError::UnexpectedEof)?
            .map_err(CompleteRoundError::Io)?;
        self.router.received_msg(incoming)?;
        Ok(())
    }

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.io.send(msg).await
    }

    fn send_many(self) -> Self::SendMany {
        SendMany { party: self }
    }

    async fn yield_now(&self) {
        self.runtime.yield_now().await
    }
}

/// Returned by [`MpcParty::send_many()`]
pub struct SendMany<M, D, R> {
    party: MpcParty<M, D, R, true>,
}

impl<M, D, E, AsyncR> super::SendMany for SendMany<M, D, AsyncR>
where
    M: ProtocolMsg + 'static,
    D: Stream<Item = Result<Incoming<M>, E>> + Unpin,
    D: Sink<Outgoing<M>, Error = E> + Unpin,
    AsyncR: runtime::AsyncRuntime,
{
    type Exec = MpcParty<M, D, AsyncR, true>;
    type Msg = <MpcParty<M, D, AsyncR> as Mpc>::Msg;
    type SendErr = <MpcParty<M, D, AsyncR> as Mpc>::SendErr;

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.party.io.feed(msg).await
    }

    async fn flush(mut self) -> Result<Self::Exec, Self::SendErr> {
        self.party.io.flush().await?;
        Ok(self.party)
    }
}

pin_project_lite::pin_project! {
    /// Merges a stream and a sink into one structure that implements both [`Stream`] and [`Sink`]
    pub struct Halves<In, Out> {
        #[pin]
        incomings: In,
        #[pin]
        outgoings: Out,
    }
}

impl<In, Out> Halves<In, Out> {
    /// Constructs `Halves`
    pub fn new(incomings: In, outgoings: Out) -> Self {
        Self {
            incomings,
            outgoings,
        }
    }

    /// Deconstructs back into halves
    pub fn into_inner(self) -> (In, Out) {
        (self.incomings, self.outgoings)
    }
}

impl<In, Out, M, E> Stream for Halves<In, Out>
where
    In: Stream<Item = Result<M, E>>,
{
    type Item = Result<M, E>;

    fn poll_next(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Option<Self::Item>> {
        let this = self.project();
        this.incomings.poll_next(cx)
    }
}

impl<In, Out, M, E> Sink<M> for Halves<In, Out>
where
    Out: Sink<M, Error = E>,
{
    type Error = E;

    fn poll_ready(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.outgoings.poll_ready(cx)
    }
    fn start_send(self: core::pin::Pin<&mut Self>, item: M) -> Result<(), Self::Error> {
        let this = self.project();
        this.outgoings.start_send(item)
    }
    fn poll_flush(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.outgoings.poll_flush(cx)
    }
    fn poll_close(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.outgoings.poll_close(cx)
    }
}

/// Error returned by [`MpcParty::complete`]
///
/// May indicate malicious behavior (e.g. adversary sent a message that aborts protocol execution)
/// or some misconfiguration of the protocol network (e.g. received a message from the round that
/// was not registered via [`Mpc::add_round`]).
#[derive(Debug, thiserror::Error)]
pub enum CompleteRoundError<ProcessErr, IoErr> {
    /// [`RoundStore`] returned an error
    ///
    /// Refer to this rounds store documentation to understand why it could fail
    #[error(transparent)]
    ProcessMsg(ProcessErr),

    /// Router error
    ///
    /// Indicates that for some reason router was not able to process a message. This can be the case of:
    /// - Router API misuse \
    ///   E.g. when received a message from the round that was not registered in the router
    /// - Improper [`RoundStore`] implementation \
    ///   Indicates that round store is not properly implemented and contains a flaw. \
    ///   For instance, this error is returned when round store indicates that it doesn't need
    ///   any more messages ([`RoundStore::wants_more`]
    ///   returns `false`), but then it didn't output anything ([`RoundStore::output`]
    ///   returns `Err(_)`)
    /// - Bug in the router
    ///
    /// This error is always related to some implementation flaw or bug: either in the code that uses
    /// the router, or in the round store implementation, or in the router itself. When implementation
    /// is correct, this error never appears. Thus, it should not be possible for the adversary to "make
    /// this error happen."
    Router(router::errors::RouterError),

    /// Receiving the next message resulted into I/O error
    Io(IoErr),
    /// Channel of incoming messages was closed before protocol completion
    UnexpectedEof,
}

impl<ProcessErr, IoErr> CompleteRoundError<ProcessErr, IoErr> {
    /// Maps I/O error
    pub fn map_io_err<E>(self, f: impl FnOnce(IoErr) -> E) -> CompleteRoundError<ProcessErr, E> {
        match self {
            CompleteRoundError::ProcessMsg(e) => CompleteRoundError::ProcessMsg(e),
            CompleteRoundError::Router(e) => CompleteRoundError::Router(e),
            CompleteRoundError::Io(e) => CompleteRoundError::Io(f(e)),
            CompleteRoundError::UnexpectedEof => CompleteRoundError::UnexpectedEof,
        }
    }
    /// Maps [`CompleteRoundError::ProcessMsg`]
    pub fn map_process_err<E>(
        self,
        f: impl FnOnce(ProcessErr) -> E,
    ) -> CompleteRoundError<E, IoErr> {
        match self {
            CompleteRoundError::ProcessMsg(e) => CompleteRoundError::ProcessMsg(f(e)),
            CompleteRoundError::Router(e) => CompleteRoundError::Router(e),
            CompleteRoundError::Io(e) => CompleteRoundError::Io(e),
            CompleteRoundError::UnexpectedEof => CompleteRoundError::UnexpectedEof,
        }
    }
}
