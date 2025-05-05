//! Primitives that process and collect messages received at certain round

use crate::Incoming;

pub use self::simple_store::{broadcast, p2p, RoundInput, RoundInputError, RoundMsgs};

mod simple_store;

/// Stores messages received at particular round
///
/// In MPC protocol, party at every round usually needs to receive up to `n` messages. `RoundsStore`
/// is a container that stores messages, it knows how many messages are expected to be received,
/// and should implement extra measures against malicious parties (e.g. prohibit message overwrite).
///
/// ## Flow
/// `RoundStore` stores received messages. Once enough messages are received, it outputs [`RoundStore::Output`].
/// In order to save received messages, [`.add_message(msg)`] is called. Then, [`.wants_more()`] tells whether more
/// messages are needed to be received. If it returned `false`, then output can be retrieved by calling [`.output()`].
///
/// [`.add_message(msg)`]: Self::add_message
/// [`.wants_more()`]: Self::wants_more
/// [`.output()`]: Self::output
///
/// ## Example
/// [`RoundInput`](super::simple_store::RoundInput) is an simple messages store. Refer to its docs to see usage examples.
pub trait RoundStore: Sized + 'static {
    /// Message type
    type Msg;
    /// Store output (e.g. `Vec<_>` of received messages)
    type Output;
    /// Store error
    type Error: core::error::Error;

    /// Adds received message to the store
    ///
    /// Returns error if message cannot be processed. Usually it means that sender behaves maliciously.
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error>;
    /// Indicates if store expects more messages to receive
    fn wants_more(&self) -> bool;
    /// Retrieves store output if enough messages are received
    ///
    /// Returns `Err(self)` if more message are needed to be received.
    ///
    /// If store indicated that it needs no more messages (ie `store.wants_more() == false`), then
    /// this function must return `Ok(_)`.
    fn output(self) -> Result<Self::Output, Self>;
}
