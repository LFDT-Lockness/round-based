//! Async runtime abstraction
//!
//! Runtime abstraction allows the MPC protocol to stay runtime-agnostic while being
//! able to use features that are common to any runtime

/// Async runtime abstraction
///
/// Abstracts async runtime like [tokio]. Currently only exposes a [yield_now](Self::yield_now)
/// function.
pub trait AsyncRuntime {
    /// Yields the execution back to the runtime
    ///
    /// If the protocol performs a long computation, it might be better for performance
    /// to split it with yield points, so the single computation does not starve other
    /// tasks.
    async fn yield_now(&self);
}

/// [Tokio](tokio)-specific async runtime
#[cfg(feature = "runtime-tokio")]
#[derive(Debug, Default)]
pub struct TokioRuntime;

#[cfg(feature = "runtime-tokio")]
impl AsyncRuntime for TokioRuntime {
    async fn yield_now(&self) {
        tokio::task::yield_now().await
    }
}

#[doc(inline)]
pub use unknown_runtime::UnknownRuntime;

/// Default runtime
#[cfg(feature = "runtime-tokio")]
#[cfg_attr(docsrs, doc(cfg(all())))]
pub type DefaultRuntime = TokioRuntime;
/// Default runtime
#[cfg(not(feature = "runtime-tokio"))]
#[cfg_attr(docsrs, doc(cfg(all())))]
pub type DefaultRuntime = UnknownRuntime;

/// Unknown async runtime
pub mod unknown_runtime {
    /// Unknown async runtime
    ///
    /// Tries to implement runtime features using generic futures code. It's better to use
    /// runtime-specific implementation.
    #[derive(Debug, Default)]
    pub struct UnknownRuntime;

    impl super::AsyncRuntime for UnknownRuntime {
        async fn yield_now(&self) {
            YieldNow(false).await
        }
    }

    /// Future for the `yield_now` function.
    pub struct YieldNow(bool);

    impl core::future::Future for YieldNow {
        type Output = ();

        fn poll(
            mut self: core::pin::Pin<&mut Self>,
            cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<Self::Output> {
            if !self.0 {
                self.0 = true;
                cx.waker().wake_by_ref();
                core::task::Poll::Pending
            } else {
                core::task::Poll::Ready(())
            }
        }
    }
}
