//! Wraps the protocol and provides sync API to execute it

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
    /// Feeds a message received from another party, and [resumes](Self::resume) the protocol execution
    ///
    /// This method can only be called if state machine returned [`ProceedResult::NeedsMoreMessages`]
    /// just before that. Calling this method when state machine did not request it may result into an
    /// error which will abort the protocol.
    ///
    /// Calling `received_msg` after protocol has finished (after it returned [`ProceedResult::Output`])
    /// returns an error.
    fn received_msg(
        &mut self,
        msg: crate::Incoming<Self::Msg>,
    ) -> ProceedResult<Self::Output, Self::Msg>;
}

/// Tells why protocol execution stopped
#[derive(Debug)]
pub enum ProceedResult<O, M> {
    /// Protocol needs provided message to be sent
    SendMsg(crate::Outgoing<M>),
    /// Protocol waits messages from other parties
    ///
    /// After the state machine requested more messages, the next call to the state machine must
    /// be [`StateMachine::received_msg`].
    NeedMoreMessages,
    /// Protocol is finised
    Output(O),
    /// Protocol yielded the execution
    ///
    /// Protocol may yield at any point by calling [`AsyncRuntime::yield_now`]. Main motivation
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
    #[displaydoc("incoming message was not picked up by the protocol")]
    IncomingMsgWasntPickedUp,
}

impl<O, M> From<Reason> for ProceedResult<O, M> {
    fn from(err: Reason) -> Self {
        ProceedResult::Error(ExecutionError(err))
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
                    return ProceedResult::NeedMoreMessages;
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

    fn received_msg(
        &mut self,
        msg: crate::Incoming<Self::Msg>,
    ) -> ProceedResult<Self::Output, Self::Msg> {
        if self.shared_state.executor_received_msg(msg).is_err() {
            return Reason::IncomingMsgWasntPickedUp.into();
        }
        self.proceed()
    }
}

/// Delivery implementation used in the state machine
pub type Delivery<M> = (Incomings<M>, Outgoings<M>);

type MpcParty<M> = crate::MpcParty<M, Delivery<M>, Runtime<M>>;

/// Wraps the protocol and provides sync API to execute it
pub fn wrap_protocol<M, F>(
    protocol: impl FnOnce(MpcParty<M>) -> F,
) -> impl StateMachine<Output = F::Output, Msg = M>
where
    F: Future,
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
