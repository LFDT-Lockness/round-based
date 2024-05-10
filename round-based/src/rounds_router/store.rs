use crate::Incoming;

/// Stores messages received at particular round
///
/// In MPC protocol, party at every round usually needs to receive up to `n` messages. `MessagesStore`
/// is a container that stores messages, it knows how many messages are expected to be received,
/// and should implement extra measures against malicious parties (e.g. prohibit message overwrite).
///
/// ## Procedure
/// `MessagesStore` stores received messages. Once enough messages are received, it outputs [`MessagesStore::Output`].
/// In order to save received messages, [`.add_message(msg)`] is called. Then, [`.wants_more()`] tells whether more
/// messages are needed to be received. If it returned `false`, then output can be retrieved by calling [`.output()`].
///
/// [`.add_message(msg)`]: Self::add_message
/// [`.wants_more()`]: Self::wants_more
/// [`.output()`]: Self::output
///
/// ## Example
/// [`RoundInput`](super::simple_store::RoundInput) is an simple messages store. Refer to its docs to see usage examples.
pub trait MessagesStore: Sized + 'static {
    /// Message type
    type Msg;
    /// Store output (e.g. `Vec<_>` of received messages)
    type Output;
    /// Store error
    type Error: crate::StdError;

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

/// Message of MPC protocol
///
/// MPC protocols typically consist of several rounds, each round has differently typed message.
/// `ProtocolMessage` and [`RoundMessage`] traits are used to examine received message: `ProtocolMessage::round`
/// determines which round message belongs to, and then `RoundMessage` trait can be used to retrieve
/// actual round-specific message.
///
/// You should derive these traits using proc macro (requires `derive` feature):
/// ```rust
/// use round_based::ProtocolMessage;
///
/// #[derive(ProtocolMessage)]
/// pub enum Message {
///     Round1(Msg1),
///     Round2(Msg2),
///     // ...
/// }
///
/// pub struct Msg1 { /* ... */ }
/// pub struct Msg2 { /* ... */ }
/// ```
///
/// This desugars into:
///
/// ```rust
/// use round_based::rounds_router::{ProtocolMessage, RoundMessage};
///
/// pub enum Message {
///     Round1(Msg1),
///     Round2(Msg2),
///     // ...
/// }
///
/// pub struct Msg1 { /* ... */ }
/// pub struct Msg2 { /* ... */ }
///
/// impl ProtocolMessage for Message {
///     fn round(&self) -> u16 {
///         match self {
///             Message::Round1(_) => 1,
///             Message::Round2(_) => 2,
///             // ...
///         }
///     }
/// }
/// impl RoundMessage<Msg1> for Message {
///     const ROUND: u16 = 1;
///     fn to_protocol_message(round_message: Msg1) -> Self {
///         Message::Round1(round_message)
///     }
///     fn from_protocol_message(protocol_message: Self) -> Result<Msg1, Self> {
///         match protocol_message {
///             Message::Round1(msg) => Ok(msg),
///             msg => Err(msg),
///         }
///     }
/// }
/// impl RoundMessage<Msg2> for Message {
///     const ROUND: u16 = 2;
///     fn to_protocol_message(round_message: Msg2) -> Self {
///         Message::Round2(round_message)
///     }
///     fn from_protocol_message(protocol_message: Self) -> Result<Msg2, Self> {
///         match protocol_message {
///             Message::Round2(msg) => Ok(msg),
///             msg => Err(msg),
///         }
///     }
/// }
/// ```
pub trait ProtocolMessage: Sized {
    /// Number of round this message originates from
    fn round(&self) -> u16;
}

/// Round message
///
/// See [`ProtocolMessage`] trait documentation.
pub trait RoundMessage<M>: ProtocolMessage {
    /// Number of the round this message belongs to
    const ROUND: u16;

    /// Converts round message into protocol message (never fails)
    fn to_protocol_message(round_message: M) -> Self;
    /// Extracts round message from protocol message
    ///
    /// Returns `Err(protocol_message)` if `protocol_message.round() != Self::ROUND`, otherwise
    /// returns `Ok(round_message)`
    fn from_protocol_message(protocol_message: Self) -> Result<M, Self>;
}
