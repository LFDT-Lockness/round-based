//! Party of MPC protocol
//!
//! [`MpcParty`] is party of MPC protocol, connected to network, ready to start carrying out the protocol.
//!
//! ```rust
//! use round_based::{Incoming, Outgoing};
//!
//! # #[derive(round_based::ProtocolMsg)]
//! # enum KeygenMsg {}
//! # struct KeyShare;
//! # struct Error;
//! # type Result<T> = std::result::Result<T, Error>;
//! # async fn doc() -> Result<()> {
//! async fn keygen<M>(party: M, i: u16, n: u16) -> Result<KeyShare>
//! where
//!     M: round_based::Mpc<Msg = KeygenMsg>
//! {
//!     // ...
//! # unimplemented!()
//! }
//! async fn connect() ->
//!     impl futures::Stream<Item = Result<Incoming<KeygenMsg>>>
//!         + futures::Sink<Outgoing<KeygenMsg>, Error = Error>
//!         + Unpin
//! {
//!     // ...
//! # round_based::_docs::fake_delivery()
//! }
//!
//! let delivery = connect().await;
//! let party = round_based::mpc::connected(delivery);
//!
//! # let (i, n) = (1, 3);
//! let keyshare = keygen(party, i, n).await?;
//! # Ok(()) }
//! ```

use crate::{
    round::{RoundInfo, RoundStore},
    Outgoing, PartyIndex,
};

pub mod party;

#[doc(no_inline)]
pub use self::party::{Halves, MpcParty};

/// Abstracts functionalities needed for MPC protocol execution
pub trait Mpc {
    /// Protocol message
    type Msg;

    /// Returned in [`Self::finish_setup`]
    type Exec: MpcExecution<Msg = Self::Msg, SendErr = Self::SendErr>;
    /// Error indicating that sending a message has failed
    type SendErr;

    /// Registers a round
    fn add_round<R>(&mut self, round: R) -> <Self::Exec as MpcExecution>::Round<R>
    where
        R: RoundStore,
        Self::Msg: RoundMsg<R::Msg>;

    /// Completes network setup
    ///
    /// Once this method is called, no more rounds can be added,
    /// but the protocol can receive and send messages.
    fn finish_setup(self) -> Self::Exec;
}

/// Abstracts functionalities needed for MPC protocol execution
pub trait MpcExecution {
    /// Witness that round was registered
    ///
    /// It is used to retrieve messages in [`MpcExecution::complete`].
    type Round<R: RoundInfo>;

    /// Protocol message
    type Msg;

    /// Error indicating that completing a round has failed
    type CompleteRoundErr<E>;
    /// Error indicating that sending a message has failed
    type SendErr;

    /// Returned by [`.send_many()`](Self::send_many)
    type SendMany: SendMany<Exec = Self, Msg = Self::Msg, SendErr = Self::SendErr>;

    /// Completes the round
    ///
    /// Waits until all messages in the round `R` are received, returns the received messages.
    async fn complete<R>(
        &mut self,
        round: Self::Round<R>,
    ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
    where
        R: RoundInfo,
        Self::Msg: RoundMsg<R::Msg>;

    /// Sends a message
    ///
    /// This method awaits until the message is sent, which might be not the best method to use if you
    /// need to send many messages at once. If it's the case, prefer using [`.send_many()`](Self::send_many).
    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr>;

    /// Sends a p2p message to another party
    ///
    /// Note: when you send many messages at once (it's most likely the case when you send a p2p message), this method
    /// is not efficient, prefer using [`.send_many()`](Self::send_many).
    async fn send_p2p(
        &mut self,
        recipient: PartyIndex,
        msg: Self::Msg,
    ) -> Result<(), Self::SendErr> {
        self.send(Outgoing::p2p(recipient, msg)).await
    }

    /// Sends a message that will be received by all parties
    ///
    /// Message will be broadcasted, but not reliably. If you need a reliable broadcast, use
    /// [`MpcExecution::reliably_broadcast`] method.
    async fn send_to_all(&mut self, msg: Self::Msg) -> Result<(), Self::SendErr> {
        self.send(Outgoing::all_parties(msg)).await
    }

    /// Reliably broadcasts a message
    ///
    /// Message will be received by all participants of the protocol. Moreover, when recipient receives a
    /// message, it will be assured (cryptographically or through other trust assumptions) that all honest
    /// participants of the protocol received the same message.
    ///
    /// It's a responsibility of a message delivery layer to provide the reliable broadcast mechanism. If
    /// it's not supported, this method returns an error. Note that not every MPC protocol requires the
    /// reliable broadcast, so it's totally normal to have a message delivery implementation that does
    /// not support it.
    async fn reliably_broadcast(&mut self, msg: Self::Msg) -> Result<(), Self::SendErr> {
        self.send(Outgoing::reliable_broadcast(msg)).await
    }

    /// Creates a buffer of outgoing messages so they can be sent all at once
    ///
    /// When you have many messages that you want to send at once, using [`.send()`](Self::send) is not efficient,
    /// as it will accumulate message delivery cost. Use this method to optimize sending many messages at once.
    fn send_many(self) -> Self::SendMany;

    /// Yields execution
    async fn yield_now(&self);
}

/// Buffer, optimized for sending many messages at once
pub trait SendMany {
    /// MPC executor, returned after successful [`.flush()`](Self::flush)
    type Exec: MpcExecution<Msg = Self::Msg, SendErr = Self::SendErr>;
    /// Protocol message
    type Msg;
    /// Error indicating that sending a message has failed
    type SendErr;

    /// Adds a message to the sending queue
    ///
    /// Similar to [`MpcExecution::send`], but possibly buffers a message until [`.flush()`](Self::flush) is
    /// called.
    ///
    /// Message may be sent within the call (e.g. if internal buffer is full), but no sending is guaranteed
    /// until [`.flush()`](Self::flush) is called.
    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr>;

    /// Adds a p2p message to the sending queue
    ///
    /// Similar to [`MpcExecution::send_p2p`], but possibly buffers a message until [`.flush()`](Self::flush) is
    /// called.
    ///
    /// Message may be sent within the call (e.g. if internal buffer is full), but no sending is guaranteed
    /// until [`.flush()`](Self::flush) is called.
    async fn send_p2p(
        &mut self,
        recipient: PartyIndex,
        msg: Self::Msg,
    ) -> Result<(), Self::SendErr> {
        self.send(Outgoing::p2p(recipient, msg)).await
    }

    /// Adds a broadcast message to the sending queue
    ///
    /// Similar to [`MpcExecution::send_to_all`], but possibly buffers a message until [`.flush()`](Self::flush) is
    /// called.
    ///
    /// Message may be sent within the call (e.g. if internal buffer is full), but no sending is guaranteed
    /// until [`.flush()`](Self::flush) is called.
    async fn send_to_all(&mut self, msg: Self::Msg) -> Result<(), Self::SendErr> {
        self.send(Outgoing::all_parties(msg)).await
    }

    /// Adds a reliable broadcast message to the sending queue
    ///
    /// Similar to [`MpcExecution::reliably_broadcast`], but possibly buffers a message until [`.flush()`](Self::flush) is
    /// called.
    ///
    /// Message may be sent within the call (e.g. if internal buffer is full), but no sending is guaranteed
    /// until [`.flush()`](Self::flush) is called.
    async fn reliably_broadcast(&mut self, msg: Self::Msg) -> Result<(), Self::SendErr> {
        self.send(Outgoing::reliable_broadcast(msg)).await
    }

    /// Flushes internal buffer by sending all messages in the queue
    async fn flush(self) -> Result<Self::Exec, Self::SendErr>;
}

/// Alias to `<<M as Mpc>::Exec as MpcExecution>::CompleteRoundErr<E>`
pub type CompleteRoundErr<M, E> = <<M as Mpc>::Exec as MpcExecution>::CompleteRoundErr<E>;

/// Message of MPC protocol
///
/// MPC protocols typically consist of several rounds, each round has differently typed message.
/// `ProtocolMsg` and [`RoundMsg`] traits are used to examine received message: `ProtocolMsg::round`
/// determines which round message belongs to, and then `RoundMessage` trait can be used to retrieve
/// actual round-specific message.
///
/// You should derive these traits using proc macro (requires `derive` feature):
/// ```rust
/// use round_based::ProtocolMsg;
///
/// #[derive(ProtocolMsg)]
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
/// use round_based::{ProtocolMsg, RoundMsg};
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
/// impl ProtocolMsg for Message {
///     fn round(&self) -> u16 {
///         match self {
///             Message::Round1(_) => 1,
///             Message::Round2(_) => 2,
///             // ...
///         }
///     }
/// }
/// impl RoundMsg<Msg1> for Message {
///     const ROUND: u16 = 1;
///     fn to_protocol_msg(round_msg: Msg1) -> Self {
///         Message::Round1(round_msg)
///     }
///     fn from_protocol_msg(protocol_msg: Self) -> Result<Msg1, Self> {
///         match protocol_msg {
///             Message::Round1(msg) => Ok(msg),
///             msg => Err(msg),
///         }
///     }
/// }
/// impl RoundMsg<Msg2> for Message {
///     const ROUND: u16 = 2;
///     fn to_protocol_msg(round_msg: Msg2) -> Self {
///         Message::Round2(round_msg)
///     }
///     fn from_protocol_msg(protocol_msg: Self) -> Result<Msg2, Self> {
///         match protocol_msg {
///             Message::Round2(msg) => Ok(msg),
///             msg => Err(msg),
///         }
///     }
/// }
/// ```
pub trait ProtocolMsg: Sized {
    /// Number of round this message originates from
    fn round(&self) -> u16;
}

/// Round message
///
/// See [`ProtocolMsg`] trait documentation.
pub trait RoundMsg<M>: ProtocolMsg {
    /// Number of the round this message belongs to
    const ROUND: u16;

    /// Converts round message into protocol message (never fails)
    fn to_protocol_msg(round_msg: M) -> Self;
    /// Extracts round message from protocol message
    ///
    /// Returns `Err(protocol_message)` if `protocol_message.round() != Self::ROUND`, otherwise
    /// returns `Ok(round_message)`
    fn from_protocol_msg(protocol_msg: Self) -> Result<M, Self>;
}

/// Construct an [`MpcParty`] that can be used to carry out MPC protocol
///
/// Accepts a channels with incoming and outgoing messages.
///
/// Alias to [`MpcParty::connected`]
pub fn connected<M, D>(delivery: D) -> MpcParty<M, D>
where
    M: ProtocolMsg + 'static,
    D: futures_util::Stream<Item = Result<crate::Incoming<M>, D::Error>> + Unpin,
    D: futures_util::Sink<Outgoing<M>> + Unpin,
{
    MpcParty::connected(delivery)
}

/// Construct an [`MpcParty`] that can be used to carry out MPC protocol
///
/// Accepts separately a channel for incoming and a channel for outgoing messages.
///
/// Alias to [`MpcParty::connected_halves`]
pub fn connected_halves<M, In, Out>(incomings: In, outgoings: Out) -> MpcParty<M, Halves<In, Out>>
where
    M: ProtocolMsg + 'static,
    In: futures_util::Stream<Item = Result<crate::Incoming<M>, Out::Error>> + Unpin,
    Out: futures_util::Sink<Outgoing<M>> + Unpin,
{
    MpcParty::connected_halves(incomings, outgoings)
}
