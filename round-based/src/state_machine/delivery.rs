use core::task::{Poll, ready};

/// Provides a stream of incoming and sink for outgoing messages
pub struct Delivery<M> {
    shared_state: super::shared_state::SharedStateRef<M>,
}

impl<M> Delivery<M> {
    pub(super) fn new(shared_state: super::shared_state::SharedStateRef<M>) -> Self {
        Self { shared_state }
    }
}

impl<M> futures_util::Stream for Delivery<M> {
    type Item = Result<crate::Incoming<M>, DeliveryErr>;

    fn poll_next(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let scheduler = ready!(self.shared_state.can_schedule());

        scheduler
            .protocol_needs_one_more_msg()
            .map(|msg| Some(Ok(msg)))
    }
}

impl<M> futures_util::Sink<crate::Outgoing<M>> for Delivery<M> {
    type Error = DeliveryErr;

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
            .map_err(|_| DeliveryErr(Reason::NotReady))
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

/// Error returned by [`Delivery`]
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct DeliveryErr(Reason);

#[derive(Debug, thiserror::Error)]
enum Reason {
    #[error("sink is not ready")]
    NotReady,
}
