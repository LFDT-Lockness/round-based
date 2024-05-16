//! Wraps the protocol defined as async function and provides sync API to execute it
//!
//! In `round_based` framework, MPC protocols are defined as async function. However, sometimes it
//! may not be possible/desirable to have async runtime which drives the futures until completion.
//! For such use-cases, we provide [`wrap_protocol`] function that wraps an MPC protocol defined as
//! async function and returns the [`StateMachine`] that exposes sync API to carry out the protocol.

mod delivery;
mod noop_waker;
mod runtime;
mod shared_state;

use core::{future::Future, task::Poll};

pub use self::{
    delivery::{Incomings, Outgoings, SendErr},
    runtime::{Runtime, YieldNow},
};

/// Provides interface to execute the protocol
pub trait StateMachine {
    /// Output of the protocol
    type Output;
    /// Message of the protocol
    type Msg;

    /// Resumes protocol execution
    ///
    /// Returns [`ProceedResult`] which will indicate, for instance, if the protocol wants to send
    /// or receive a message, or if it's finished.
    ///
    /// Calling `proceed` after protocol has finished (after it returned [`ProceedResult::Output`])
    /// returns an error.
    fn proceed(&mut self) -> ProceedResult<Self::Output, Self::Msg>;
    /// Saves received message to be picked up by the state machine on the next [`proceed`](Self::proceed) invocation
    ///
    /// This method should only be called if state machine returned [`ProceedResult::NeedsOneMoreMessage`] on previous
    /// invocation of [`proceed`](Self::proceed) method. Calling this method when state machine did not request it
    /// may return error.
    ///
    /// Calling this method must be followed up by calling [`proceed`](Self::proceed). Do not invoke this method
    /// more than once in a row, even if you have available messages received from other parties. Instead, you
    /// should call this method, then call `proceed`, and only if it returned [`ProceedResult::NeedsOneMoreMessage`]
    /// you can call `received_msg` again.
    fn received_msg(
        &mut self,
        msg: crate::Incoming<Self::Msg>,
    ) -> Result<(), crate::Incoming<Self::Msg>>;
}

/// Tells why protocol execution stopped
#[must_use = "ProceedResult must be used to correcty carry out the state machine"]
pub enum ProceedResult<O, M> {
    /// Protocol needs provided message to be sent
    SendMsg(crate::Outgoing<M>),
    /// Protocol needs one more message to be received
    ///
    /// After the state machine requested one more message, the next call to the state machine must
    /// be [`StateMachine::received_msg`].
    NeedsOneMoreMessage,
    /// Protocol is finised
    Output(O),
    /// Protocol yielded the execution
    ///
    /// Protocol may yield at any point by calling `AsyncRuntime::yield_now`. Main motivation
    /// for yielding is to break a long computation into smaller parts, so proceeding state
    /// machine doesn't take too long.
    ///
    /// When protocol yields, you can resume the execution by calling [`proceed`](StateMachine::proceed)
    /// immediately.
    Yielded,
    /// State machine failed to carry out the protocol
    ///
    /// Error likely means that either state machine is misused (e.g. when [`proceed`](StateMachine::proceed)
    /// is called after protocol is finished) or protocol implementation is not supported by state machine
    /// executor (e.g. it polls unknown future).
    Error(ExecutionError),
}

impl<O, M> core::fmt::Debug for ProceedResult<O, M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProceedResult::SendMsg(_) => f.write_str("SendMsg"),
            ProceedResult::NeedsOneMoreMessage => f.write_str("NeedsOneMoreMessage"),
            ProceedResult::Output(_) => f.write_str("Output"),
            ProceedResult::Yielded => f.write_str("Yielded"),
            ProceedResult::Error(_) => f.write_str("Error"),
        }
    }
}

/// Error type which indicates that state machine failed to carry out the protocol
#[derive(Debug, displaydoc::Display)]
#[displaydoc("{0}")]
pub struct ExecutionError(Reason);

#[derive(Debug, displaydoc::Display)]
enum Reason {
    #[displaydoc("resuming state machine when protocol is already finished")]
    Exhausted,
    #[displaydoc("protocol polls unknown (unsupported) future")]
    PollingUnknownFuture,
}

impl<O, M> From<Reason> for ProceedResult<O, M> {
    fn from(err: Reason) -> Self {
        ProceedResult::Error(ExecutionError(err))
    }
}
impl From<Reason> for ExecutionError {
    fn from(err: Reason) -> Self {
        ExecutionError(err)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ExecutionError {}

struct StateMachineImpl<O, M, F: Future<Output = O>> {
    shared_state: shared_state::SharedStateRef<M>,
    exhausted: bool,
    future: core::pin::Pin<alloc::boxed::Box<F>>,
}

impl<O, M, F> StateMachine for StateMachineImpl<O, M, F>
where
    F: Future<Output = O>,
{
    type Output = O;
    type Msg = M;

    fn proceed(&mut self) -> ProceedResult<Self::Output, Self::Msg> {
        if self.exhausted {
            return Reason::Exhausted.into();
        }
        let future = self.future.as_mut();
        let waker = noop_waker::noop_waker();
        let mut cx = core::task::Context::from_waker(&waker);
        match future.poll(&mut cx) {
            Poll::Ready(output) => {
                self.exhausted = true;
                ProceedResult::Output(output)
            }
            Poll::Pending => {
                // underlying future may `await` only on either:
                // 1. Flushing outgoing message
                // 2. Waiting for incoming message
                // 3. Yielding

                // Check if it's flushing outgoing message:
                if let Some(outgoing_msg) = self.shared_state.executor_takes_outgoing_msg() {
                    return ProceedResult::SendMsg(outgoing_msg);
                }

                // Check if it's waiting for a new message
                if self.shared_state.protocol_wants_more_messages() {
                    return ProceedResult::NeedsOneMoreMessage;
                }

                // Check if protocol yielded
                if self.shared_state.executor_reads_and_resets_yielded_flag() {
                    return ProceedResult::Yielded;
                }

                // If none of above conditions are met, then protocol is polling
                // a future which we do not recognize
                Reason::PollingUnknownFuture.into()
            }
        }
    }

    fn received_msg(&mut self, msg: crate::Incoming<Self::Msg>) -> Result<(), crate::Incoming<M>> {
        self.shared_state.executor_received_msg(msg)
    }
}

/// Delivery implementation used in the state machine
pub type Delivery<M> = (Incomings<M>, Outgoings<M>);

/// MpcParty instantiated with state machine implementation of delivery and async runtime
pub type MpcParty<M> = crate::MpcParty<M, Delivery<M>, Runtime<M>>;

/// Wraps the protocol and provides sync API to execute it
///
/// Protocol is an async function that takes [`MpcParty`] as input. `MpcParty` contains
/// channels (of incoming and outgoing messages) that protocol is expected to use, and
/// a [`Runtime`]. Protocol is only allowed to `.await` on futures provided in `MpcParty`,
/// such as polling next message from provided steam of incoming messages. If protocol
/// polls an unknown future, executor won't know what to do with that, the protocol will
/// be aborted and error returned.
pub fn wrap_protocol<'a, M, F>(
    protocol: impl FnOnce(MpcParty<M>) -> F,
) -> impl StateMachine<Output = F::Output, Msg = M> + 'a
where
    F: Future + 'a,
    M: 'static,
{
    let shared_state = shared_state::SharedStateRef::new();
    let incomings = Incomings::new(shared_state.clone());
    let outgoings = Outgoings::new(shared_state.clone());
    let delivery = (incomings, outgoings);
    let runtime = Runtime::new(shared_state.clone());

    let future = protocol(crate::MpcParty::connected(delivery).set_runtime(runtime));
    let future = alloc::boxed::Box::pin(future);

    StateMachineImpl {
        shared_state,
        exhausted: false,
        future,
    }
}
