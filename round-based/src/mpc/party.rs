use futures_util::{Sink, SinkExt, Stream, StreamExt};

use crate::{round::RoundStore, Incoming, Outgoing};

use super::{rounds_router, runtime, Mpc, MpcExecution, ProtocolMsg, RoundMsg};

/// MPC engine, carries out the protocol
///
/// Can be constructed via [`MpcParty::connected`] or [`MpcParty::connected_halves`], which wraps
/// a channel of incoming and outgoing messages, and implements additional logic on top of this
/// to facilitate the MPC protocol execution, such as routing incoming messages between round
/// stores.
///
/// Implements [`Mpc`] and [`MpcExecution`].
pub struct MpcParty<M, D, R = runtime::DefaultRuntime, const SETUP_COMPLETE: bool = false> {
    router: rounds_router::RoundsRouter<M>,
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
            router: rounds_router::RoundsRouter::new(),
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
    type Round<R> = rounds_router::Round<R>;

    type Msg = M;

    type CompleteRoundErr<E> = WithIo<IoErr, rounds_router::errors::CompleteRoundError<E>>;

    type SendErr = IoErr;

    async fn complete<R>(
        &mut self,
        mut round: Self::Round<R>,
    ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
    where
        R: RoundStore,
        Self::Msg: RoundMsg<R::Msg>,
    {
        // Check if round is already completed
        round = match self.router.complete_round(round) {
            Ok(output) => return output.map_err(WithIo::Other),
            Err(w) => w,
        };

        // Round is not completed - we need more messages
        loop {
            let incoming = self
                .io
                .next()
                .await
                .ok_or(WithIo::UnexpectedEof)?
                .map_err(WithIo::Io)?;
            self.router
                .received_msg(incoming)
                .map_err(|err| WithIo::Other(err.into()))?;

            // Check if round was just completed
            round = match self.router.complete_round(round) {
                Ok(output) => return output.map_err(WithIo::Other),
                Err(w) => w,
            };
        }
    }

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.io.send(msg).await
    }

    async fn yield_now(&self) {
        self.runtime.yield_now().await
    }
}

/// Error indicating that either `IoErr` occurred, or `OtherErr`
#[derive(Debug, thiserror::Error)]
pub enum WithIo<IoErr, OtherErr> {
    /// IO error
    #[error(transparent)]
    Io(IoErr),
    /// Unexpected EOF
    #[error("unexpected eof")]
    UnexpectedEof,
    /// Other error
    #[error(transparent)]
    Other(OtherErr),
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
