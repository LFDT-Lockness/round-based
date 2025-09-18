use core::task::{Poll, ready};

/// State machine runtime
pub struct Runtime<M> {
    shared_state: super::shared_state::SharedStateRef<M>,
}

impl<M> Runtime<M> {
    pub(super) fn new(shared_state: super::shared_state::SharedStateRef<M>) -> Self {
        Self { shared_state }
    }
}

impl<M> crate::mpc::party::AsyncRuntime for Runtime<M> {
    async fn yield_now(&self) {
        YieldNow {
            shared_state: self.shared_state.clone(),
            yielded: false,
        }
        .await
    }
}

/// Future returned by [`runtime.yield_now()`](Runtime)
pub struct YieldNow<M> {
    shared_state: super::shared_state::SharedStateRef<M>,
    yielded: bool,
}

impl<M> core::future::Future for YieldNow<M> {
    type Output = ();

    fn poll(
        mut self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if !self.yielded {
            let scheduler = ready!(self.shared_state.can_schedule());
            scheduler.protocol_yields();
            self.yielded = true;
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}
