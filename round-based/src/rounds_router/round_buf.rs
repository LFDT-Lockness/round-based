//! Buffers of messages for [buffered rounds](super::RoundsRouterBuilder::add_buffered_round)

use alloc::collections::VecDeque;
use core::convert::Infallible;

use crate::Incoming;

/// Buffers messages received at particular round until the protocol retrieves them
///
/// Unlike [`MessagesStore`](super::MessagesStore), which accumulates messages until the round is
/// complete, `RoundBuf` only holds messages that were received before the protocol asked for
/// them via [`RoundsRouter::next_msg_in`](super::RoundsRouter::next_msg_in). Protocol then
/// processes messages manually, one-by-one.
///
/// `RoundBuf` encapsulates how the messages are buffered: how many messages can be buffered, and
/// in which order they're given back to the protocol. Provided implementations,
/// [`RoundBufBounded`] and [`RoundBufUnbounded`], are fair FIFO queues, but the buffer may
/// implement any ordering that makes sense to the protocol.
pub trait RoundBuf: 'static {
    /// Message type
    type Msg;
    /// Buffer error
    type Error: core::error::Error;

    /// Adds received message to the buffer
    ///
    /// Returns error if message cannot be buffered. Once error is returned, no more messages
    /// are added to the buffer: protocol can retrieve messages that are already buffered, and
    /// then [`RoundsRouter::next_msg_in`](super::RoundsRouter::next_msg_in) returns the error.
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error>;
    /// Takes next message out of the buffer
    ///
    /// Returns `None` if there are no buffered messages.
    fn next_message(&mut self) -> Option<Incoming<Self::Msg>>;
}

/// FIFO buffer that holds unlimited amount of messages
///
/// Note that amount of buffered messages is controlled by other parties: malicious party may
/// keep sending messages to exhaust the memory. Consider using [`RoundBufBounded`] if that's a
/// concern.
#[derive(Debug, Clone)]
pub struct RoundBufUnbounded<M> {
    messages: VecDeque<Incoming<M>>,
}

impl<M> RoundBufUnbounded<M> {
    /// Constructs an empty buffer
    pub fn new() -> Self {
        Self {
            messages: VecDeque::new(),
        }
    }
}

impl<M> Default for RoundBufUnbounded<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: 'static> RoundBuf for RoundBufUnbounded<M> {
    type Msg = M;
    type Error = Infallible;

    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        self.messages.push_back(msg);
        Ok(())
    }
    fn next_message(&mut self) -> Option<Incoming<Self::Msg>> {
        self.messages.pop_front()
    }
}

/// FIFO buffer that holds at most `capacity` messages
///
/// Receiving a message when the buffer is full is a hard error: protocol can still retrieve
/// messages that are already buffered, and then the round is aborted with [`RoundBufOverflow`].
///
/// ## Example
/// ```rust
/// # use round_based::rounds_router::round_buf::{RoundBuf, RoundBufBounded};
/// # use round_based::{Incoming, MessageType};
/// let mut buf = RoundBufBounded::<&'static str>::new(1);
/// buf.add_message(Incoming{
///     id: 0,
///     sender: 0,
///     msg_type: MessageType::Broadcast,
///     msg: "first message",
/// }).unwrap();
/// // buffer is full
/// buf.add_message(Incoming{
///     id: 1,
///     sender: 2,
///     msg_type: MessageType::Broadcast,
///     msg: "second message",
/// }).unwrap_err();
///
/// assert_eq!(buf.next_message().unwrap().msg, "first message");
/// assert!(buf.next_message().is_none());
/// ```
#[derive(Debug, Clone)]
pub struct RoundBufBounded<M> {
    capacity: usize,
    messages: VecDeque<Incoming<M>>,
}

impl<M> RoundBufBounded<M> {
    /// Constructs an empty buffer that can hold at most `capacity` messages
    ///
    /// Every message of the round goes through the buffer, even if it's received when the
    /// protocol is already waiting for it. Therefore, buffer with `capacity=0` accepts no
    /// messages.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            messages: VecDeque::new(),
        }
    }
}

impl<M: 'static> RoundBuf for RoundBufBounded<M> {
    type Msg = M;
    type Error = RoundBufOverflow;

    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        if self.messages.len() >= self.capacity {
            return Err(RoundBufOverflow {
                capacity: self.capacity,
            });
        }
        self.messages.push_back(msg);
        Ok(())
    }
    fn next_message(&mut self) -> Option<Incoming<Self::Msg>> {
        self.messages.pop_front()
    }
}

/// Error indicating that [`RoundBufBounded`] received a message when it was full
#[derive(Debug, Clone, thiserror::Error)]
#[error("buffer is full: it already holds {capacity} messages")]
pub struct RoundBufOverflow {
    capacity: usize,
}
