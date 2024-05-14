use core::task::Poll;

pub struct SharedStateRef<M>(alloc::rc::Rc<core::cell::RefCell<SharedState<M>>>);

struct SharedState<M> {
    incoming_msg: Option<crate::Incoming<M>>,
    outgoing_msg: Option<crate::Outgoing<M>>,
    wants_recv_msg: bool,
    wants_send_msg: bool,
    yielded: bool,
}

impl<M> SharedState<M> {
    pub fn new() -> Self {
        Self {
            incoming_msg: None,
            outgoing_msg: None,
            wants_recv_msg: false,
            wants_send_msg: false,
            yielded: false,
        }
    }
}

impl<M> SharedStateRef<M> {
    pub fn new() -> Self {
        Self(alloc::rc::Rc::new(core::cell::RefCell::new(
            SharedState::new(),
        )))
    }

    /// Any protocol-initated work (like flushing message to be sent, receiving message, etc.) can
    /// only be scheduled when there was no other task scheduled.
    ///
    /// This method checks whether a task can be scheduled, and returns [`CanSchedule`] which
    /// then can be used to schedule one task. If some other task is already scheduled, it returns
    /// `Poll::Pending`.
    pub fn can_schedule(&self) -> core::task::Poll<CanSchedule<&Self>> {
        let s = self.0.borrow();
        let can_poll = !s.wants_recv_msg && !s.wants_send_msg && !s.yielded;

        if can_poll {
            core::task::Poll::Ready(CanSchedule(self))
        } else {
            core::task::Poll::Pending
        }
    }

    /// Puts a message to be sent into sending slot
    ///
    /// Returns `Err(msg)` if slot is already occupied (which means that flushing must be done
    /// before sending a message).
    ///
    /// Note that it does not schedules that message to be sent. To do so, you need to schedule
    /// flushing via [`CanSchedule::protocol_flushes_outgoing_msg`]
    pub fn protocol_saves_msg_to_be_sent(
        &self,
        msg: crate::Outgoing<M>,
    ) -> Result<(), crate::Outgoing<M>> {
        let mut s = self.0.borrow_mut();
        if s.outgoing_msg.is_some() {
            return Err(msg);
        }
        s.outgoing_msg = Some(msg);
        Ok(())
    }

    /// Takes outgoing message to be sent, but only if flushing was requested
    pub fn executor_takes_outgoing_msg(&self) -> Option<crate::Outgoing<M>> {
        let mut s = self.0.borrow_mut();
        if s.wants_send_msg {
            debug_assert!(s.outgoing_msg.is_some());
            s.wants_send_msg = false;
            s.outgoing_msg.take()
        } else {
            None
        }
    }

    /// Checks if protocol has indicated that it waits for a new message from other parties.
    /// Does not mutate the state
    pub fn protocol_wants_more_messages(&self) -> bool {
        let s = self.0.borrow();
        s.wants_recv_msg
    }

    /// Checks if protocol yielded. Sets `yield_now = false`
    pub fn executor_reads_and_resets_yielded_flag(&self) -> bool {
        let mut s = self.0.borrow_mut();
        let y = s.yielded;
        s.yielded = false;
        y
    }

    /// Saves received msg to be picked up by the `Incomings`. Sets `wants_recv_msg = false`
    pub fn executor_received_msg(&self, msg: crate::Incoming<M>) -> Result<(), crate::Incoming<M>> {
        let mut s = self.0.borrow_mut();
        if s.incoming_msg.is_some() {
            return Err(msg);
        }
        s.incoming_msg = Some(msg);
        s.wants_recv_msg = false;
        Ok(())
    }
}

impl<M> Clone for SharedStateRef<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> core::fmt::Debug for SharedState<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SharedState")
            .field("incoming_msg_present", &self.incoming_msg.is_some())
            .field("outgoing_msg_present", &self.outgoing_msg.is_some())
            .field("wants_recv_msg", &self.wants_recv_msg)
            .field("wants_recv_msg", &self.wants_recv_msg)
            .field("yielded", &self.yielded)
            .finish()
    }
}
impl<M> core::fmt::Debug for SharedStateRef<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

/// Type guard witnessing that shared state has no tasks scheduled.
/// Can be used to schedule only one task. Can be obtained by calling
/// [`SharedStateRef::can_schedule`].
pub struct CanSchedule<T>(T);

impl<'a, M> CanSchedule<&'a SharedStateRef<M>> {
    fn borrow_mut(&self) -> core::cell::RefMut<SharedState<M>> {
        self.0 .0.borrow_mut()
    }

    /// Flushes slot of outgoing message
    ///
    /// Returns `Poll::Ready(())` when slot is emptied
    pub fn protocol_flushes_outgoing_msg(self) -> Poll<()> {
        let mut s = self.borrow_mut();
        if s.outgoing_msg.is_some() {
            s.wants_send_msg = true;
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }

    /// Indicated that the protocol needs more messages from other parties
    pub fn protocol_needs_more_msgs(self) -> Poll<crate::Incoming<M>> {
        let mut s = self.borrow_mut();
        match s.incoming_msg.take() {
            Some(msg) => Poll::Ready(msg),
            None => {
                s.wants_recv_msg = true;
                Poll::Pending
            }
        }
    }

    /// Indicates that protocol yielded
    pub fn protocol_yields(self) {
        let mut s = self.borrow_mut();
        s.yielded = true;
    }
}

#[cfg(test)]
mod test {
    use core::task::Poll;

    use crate::{MessageDestination, Outgoing};

    use super::SharedStateRef;

    #[test]
    fn send_msg() {
        let shared_state = SharedStateRef::<u32>::new();

        let outgoings_state = shared_state.clone();
        let executor_state = shared_state;

        let msg = Outgoing {
            recipient: MessageDestination::AllParties,
            msg: 1,
        };
        outgoings_state
            .protocol_saves_msg_to_be_sent(msg)
            .expect("msg slot isn't empty");

        let Poll::Ready(scheduler) = outgoings_state.can_schedule() else {
            panic!("can't schedule");
        };

        let Poll::Pending = scheduler.protocol_flushes_outgoing_msg() else {
            panic!("flushing resolved too early");
        };

        let msg_actual = executor_state.executor_takes_outgoing_msg().unwrap();
        assert_eq!(msg, msg_actual);

        let Poll::Ready(scheduler) = outgoings_state.can_schedule() else {
            panic!("can't schedule");
        };
        let Poll::Ready(()) = scheduler.protocol_flushes_outgoing_msg() else {
            panic!("flushing must be done at this point");
        };
    }

    #[test]
    fn task_cannot_be_scheduled_when_another_task_is_scheduled() {
        let try_obtain_lock_and_fail = |shared_state: &SharedStateRef<u32>| {
            let Poll::Pending = shared_state.can_schedule() else {
                panic!("lock must not be obtained");
            };
        };

        // When message is being flushed
        {
            let shared_state = SharedStateRef::new();
            shared_state
                .protocol_saves_msg_to_be_sent(Outgoing {
                    recipient: MessageDestination::AllParties,
                    msg: 1,
                })
                .expect("msg slot isn't empty");
            let Poll::Ready(scheduler) = shared_state.can_schedule() else {
                panic!("can't schedule");
            };
            let Poll::Pending = scheduler.protocol_flushes_outgoing_msg() else {
                panic!("flushing resolved too early")
            };

            // Now that flushing is scheduled, we can't schedule any more tasks
            try_obtain_lock_and_fail(&shared_state);
        }

        // When message is requested to be received
        {
            let shared_state = SharedStateRef::new();
            let Poll::Ready(scheduler) = shared_state.can_schedule() else {
                panic!("can't schedule");
            };
            let Poll::Pending = scheduler.protocol_needs_more_msgs() else {
                panic!("receiving resolved too early")
            };

            // Now that receiving is scheduled, we can't schedule any more tasks
            try_obtain_lock_and_fail(&shared_state);
        }

        // When protocol yielded
        {
            let shared_state = SharedStateRef::new();
            let Poll::Ready(scheduler) = shared_state.can_schedule() else {
                panic!("can't schedule");
            };
            scheduler.protocol_yields();

            // Now that yielding is scheduled, we can't schedule any more tasks
            try_obtain_lock_and_fail(&shared_state);
        }
    }
}
