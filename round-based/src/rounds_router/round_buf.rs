//! Buffers of messages for [buffered rounds](super::RoundsRouterBuilder::add_buffered_round)

use alloc::{collections::VecDeque, vec::Vec};
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

/// FIFO buffer that accepts at most `limit_per_party` messages from each party
///
/// Each party can send no more than `limit_per_party` messages during the whole round. If party
/// sent that many messages, any new message from it will be discarded without reporting any error
/// to the protocol. The fact of hitting the limit is logged via `tracing::warn`.
///
/// Therefore, the buffer never holds more than `parties_n * limit_per_party` messages.
///
/// Receiving a message from a party with index `i >= parties_n` is a hard error: the round is
/// aborted with [`RoundBufBoundedError`].
///
/// ## Example
/// ```rust
/// # use round_based::rounds_router::round_buf::{RoundBuf, RoundBufBounded};
/// # use round_based::{Incoming, MessageType};
/// let mut buf = RoundBufBounded::<&'static str>::new(2, 1);
/// buf.add_message(Incoming{
///     id: 0,
///     sender: 0,
///     msg_type: MessageType::Broadcast,
///     msg: "message from party 0",
/// }).unwrap();
/// // Party 0 hit the limit, its messages are silently discarded
/// buf.add_message(Incoming{
///     id: 1,
///     sender: 0,
///     msg_type: MessageType::Broadcast,
///     msg: "another message from party 0 that is over the limit",
/// }).unwrap();
/// // Messages from other parties are still being accepted
/// buf.add_message(Incoming{
///     id: 2,
///     sender: 1,
///     msg_type: MessageType::Broadcast,
///     msg: "message from party 1",
/// }).unwrap();
///
/// // All received messages in FIFO order, except the discarded ones
/// assert_eq!(buf.next_message().unwrap().msg, "message from party 0");
/// assert_eq!(buf.next_message().unwrap().msg, "message from party 1");
/// assert!(buf.next_message().is_none());
/// ```
#[derive(Debug, Clone)]
pub struct RoundBufBounded<M> {
    /// `left_msgs[i]` is amount of messages that can still be accepted from party `i`
    left_msgs: Vec<usize>,
    messages: VecDeque<Incoming<M>>,
}

impl<M> RoundBufBounded<M> {
    /// Constructs an empty buffer that accepts at most `limit_per_party` messages from each of
    /// `parties_n` parties
    ///
    /// Every message of the round goes through the buffer, even if it's received when the
    /// protocol is already waiting for it. Therefore, buffer with `limit_per_party=0` accepts
    /// no messages.
    pub fn new(parties_n: u16, limit_per_party: usize) -> Self {
        Self {
            left_msgs: alloc::vec![limit_per_party; usize::from(parties_n)],
            messages: VecDeque::new(),
        }
    }
}

impl<M: 'static> RoundBuf for RoundBufBounded<M> {
    type Msg = M;
    type Error = RoundBufBoundedError;

    fn add_message(&mut self, incoming: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        let parties_n = self.left_msgs.len();
        let left_msgs = self.left_msgs.get_mut(usize::from(incoming.sender)).ok_or(
            RoundBufBoundedReason::UnknownSender {
                i: incoming.sender,
                n: parties_n,
            },
        )?;
        *left_msgs = match left_msgs.checked_sub(1) {
            Some(x) => x,
            None => {
                tracing::warn!(
                    sender = incoming.sender,
                    "party sent too many messages and hit `RoundBufBounded::new(_, limit_per_party)`; \
                    message is discarded"
                );
                return Ok(());
            }
        };
        self.messages.push_back(incoming);
        Ok(())
    }
    fn next_message(&mut self) -> Option<Incoming<Self::Msg>> {
        self.messages.pop_front()
    }
}

/// Error indicating that [`RoundBufBounded`] received a message from unknown party
///
/// Note that party sending more messages than allowed is not an error: such messages are
/// discarded.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct RoundBufBoundedError(#[from] RoundBufBoundedReason);

#[derive(Debug, thiserror::Error)]
enum RoundBufBoundedReason {
    #[error("received a message from unknown party i={i} (n={n})")]
    UnknownSender { i: u16, n: usize },
}
