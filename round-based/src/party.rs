//! Party of MPC protocol
//!
//! [`MpcParty`] is party of MPC protocol, connected to network, ready to start carrying out the protocol.
//!
//! ```rust
//! use round_based::{Mpc, MpcParty, Delivery, PartyIndex};
//!
//! # struct KeygenMsg;
//! # struct KeyShare;
//! # struct Error;
//! # type Result<T> = std::result::Result<T, Error>;
//! # async fn doc() -> Result<()> {
//! async fn keygen<M>(party: M, i: PartyIndex, n: u16) -> Result<KeyShare>
//! where
//!     M: Mpc<ProtocolMessage = KeygenMsg>
//! {
//!     // ...
//! # unimplemented!()
//! }
//! async fn connect() -> impl Delivery<KeygenMsg> {
//!     // ...
//! # round_based::_docs::fake_delivery()
//! }
//!
//! let delivery = connect().await;
//! let party = MpcParty::connected(delivery);
//!
//! # let (i, n) = (1, 3);
//! let keyshare = keygen(party, i, n).await?;
//! # Ok(()) }
//! ```

use phantom_type::PhantomType;

use crate::delivery::Delivery;
use crate::runtime::{self, AsyncRuntime};
use crate::StdError;

/// Party of MPC protocol (trait)
///
/// [`MpcParty`] is the only struct that implement this trait. Motivation to have this trait is to fewer amount of
/// generic bounds that are needed to be specified.
///
/// Typical usage of this trait when implementing MPC protocol:
///
/// ```rust
/// use round_based::{Mpc, MpcParty, PartyIndex};
///
/// # struct Msg;
/// async fn keygen<M>(party: M, i: PartyIndex, n: u16)
/// where
///     M: Mpc<ProtocolMessage = Msg>
/// {
///     let MpcParty{ delivery, .. } = party.into_party();
///     // ...
/// }
/// ```
///
/// If we didn't have this trait, generics would be less readable:
/// ```rust
/// use round_based::{MpcParty, Delivery, runtime::AsyncRuntime, PartyIndex};
///
/// # struct Msg;
/// async fn keygen<D, R>(party: MpcParty<Msg, D, R>, i: PartyIndex, n: u16)
/// where
///     D: Delivery<Msg>,
///     R: AsyncRuntime
/// {
///     // ...
/// }
/// ```
pub trait Mpc: internal::Sealed {
    /// MPC message
    type ProtocolMessage;
    /// Transport layer implementation
    type Delivery: Delivery<
        Self::ProtocolMessage,
        SendError = Self::SendError,
        ReceiveError = Self::ReceiveError,
    >;
    /// Async runtime
    type Runtime: AsyncRuntime;

    /// Sending message error
    type SendError: StdError + Send + Sync + 'static;
    /// Receiving message error
    type ReceiveError: StdError + Send + Sync + 'static;

    /// Converts into [`MpcParty`]
    fn into_party(self) -> MpcParty<Self::ProtocolMessage, Self::Delivery, Self::Runtime>;
}

mod internal {
    pub trait Sealed {}
}

/// Party of MPC protocol
#[non_exhaustive]
pub struct MpcParty<M, D, R = runtime::DefaultRuntime> {
    /// Defines transport layer
    pub delivery: D,
    /// Defines how computationally heavy tasks should be handled
    pub runtime: R,
    _msg: PhantomType<M>,
}

impl<M, D> MpcParty<M, D>
where
    D: Delivery<M>,
{
    /// Party connected to the network
    ///
    /// Takes the delivery object determining how to deliver/receive other parties' messages
    pub fn connected(delivery: D) -> Self {
        Self {
            delivery,
            runtime: Default::default(),
            _msg: PhantomType::new(),
        }
    }
}

impl<M, D, X> MpcParty<M, D, X>
where
    D: Delivery<M>,
{
    /// Specifies a [async runtime](runtime)
    pub fn set_runtime<R>(self, runtime: R) -> MpcParty<M, D, R>
    where
        R: AsyncRuntime,
    {
        MpcParty {
            delivery: self.delivery,
            runtime,
            _msg: self._msg,
        }
    }
}

impl<M, D, B> internal::Sealed for MpcParty<M, D, B> {}

impl<M, D, R> Mpc for MpcParty<M, D, R>
where
    D: Delivery<M>,
    D::SendError: StdError + Send + Sync + 'static,
    D::ReceiveError: StdError + Send + Sync + 'static,
    R: AsyncRuntime,
{
    type ProtocolMessage = M;
    type Delivery = D;
    type Runtime = R;

    type SendError = D::SendError;
    type ReceiveError = D::ReceiveError;

    fn into_party(self) -> MpcParty<Self::ProtocolMessage, Self::Delivery, Self::Runtime> {
        self
    }
}
