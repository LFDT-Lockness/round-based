use core::task::{ready, Poll};

/// Stream of incoming messages
pub struct Incomings<M> {
    shared_state: super::shared_state::SharedStateRef<M>,
}

impl<M> Incomings<M> {
    pub(super) fn new(shared_state: super::shared_state::SharedStateRef<M>) -> Self {
        Self { shared_state }
    }
}

impl<M> crate::Stream for Incomings<M> {
    type Item = Result<crate::Incoming<M>, core::convert::Infallible>;

    fn poll_next(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let scheduler = ready!(self.shared_state.can_schedule());

        scheduler
            .protocol_needs_more_msgs()
            .map(|msg| Some(Ok(msg)))
    }
}

/// Sink for outgoing messages
pub struct Outgoings<M> {
    shared_state: super::shared_state::SharedStateRef<M>,
}

impl<M> Outgoings<M> {
    pub(super) fn new(shared_state: super::shared_state::SharedStateRef<M>) -> Self {
        Self { shared_state }
    }
}

impl<'a, M> crate::Sink<crate::Outgoing<M>> for Outgoings<M> {
    type Error = SendErr;

    fn poll_ready(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let scheduler = ready!(self.shared_state.can_schedule());
        scheduler.protocol_flushes_outgoing_msg().map(Ok)
    }

    fn start_send(
        self: core::pin::Pin<&mut Self>,
        msg: crate::Outgoing<M>,
    ) -> Result<(), Self::Error> {
        self.shared_state
            .protocol_saves_msg_to_be_sent(msg)
            .map_err(|_| SendErr(SendErrReason::NotReady))
    }

    fn poll_flush(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let scheduler = ready!(self.shared_state.can_schedule());
        scheduler.protocol_flushes_outgoing_msg().map(Ok)
    }

    fn poll_close(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Error returned by [`Outgoings`] sink
#[derive(Debug, displaydoc::Display)]
#[displaydoc("{0}")]
pub struct SendErr(SendErrReason);

#[derive(Debug, displaydoc::Display)]
enum SendErrReason {
    #[displaydoc("sink is not ready")]
    NotReady,
}

#[cfg(feature = "std")]
impl std::error::Error for SendErr {}
