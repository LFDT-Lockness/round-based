//! Fully async simulation
//!
//! Simulation provided in a [parent module](super) should be used in most cases. It works
//! by converting all parties (defined as async functions) into [state machines](crate::state_machine),
//! which has certain limitations. In particular, the protocol cannot await on any futures that
//! aren't provided by [`MpcParty`], for instance, awaiting on the timer will cause a simulation error.
//!
//! We suggest to avoid awaiting on the futures that aren't provided by `MpcParty` in the MPC protocol
//! implementation as it likely makes it runtime-dependent. However, if you do ultimately need to
//! do that, then you can't use regular simulation for the tests.
//!
//! This module provides fully async simulation built for tokio runtime, so the protocol can await
//! on any futures supported by the tokio.
//!
//! ## Limitations
//! To implement simulated [network](Network), we used [`tokio::sync::broadcast`] channels, which
//! have internal buffer of stored messages, and once simulated network receives more messages than
//! internal buffer can fit, some of the parties will not receive some of the messages, which will
//! lead to execution error.
//!
//! By default, internal buffer is preallocated to fit 500 messages, which should be more than
//! sufficient for simulating protocols with small amount of parties (say, < 10).
//!
//! If you need to preallocate bigger buffer, use [`Network::with_capacity`].
//!
//! ## Example
//! Entry point to the simulation are [`run`] and [`run_with_setup`] functions
//!
//! ```rust,no_run
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! use round_based::{Mpc, PartyIndex};
//!
//! # type Result<T, E = ()> = std::result::Result<T, E>;
//! # type Randomness = [u8; 32];
//! # #[derive(round_based::ProtocolMsg, Clone)]
//! # enum Msg {}
//! // Any MPC protocol you want to test
//! pub async fn protocol_of_random_generation<M>(
//!     party: M,
//!     i: PartyIndex,
//!     n: u16
//! ) -> Result<Randomness>
//! where
//!     M: Mpc<Msg = Msg>
//! {
//!     // ...
//! # todo!()
//! }
//!
//! let n = 3;
//!
//! let output = round_based::sim::async_env::run(
//!     n,
//!     |i, party| protocol_of_random_generation(party, i, n),
//! )
//! .await
//! // unwrap `Result`s
//! .expect_ok()
//! // check that all parties produced the same response
//! .expect_eq();
//!
//! println!("Output randomness: {}", hex::encode(output));
//! # }  
//! ```
use alloc::sync::Arc;
use core::{
    future::Future,
    pin::Pin,
    sync::atomic::AtomicU64,
    task::ready,
    task::{Context, Poll},
};

use futures_util::{Sink, Stream};
use tokio::sync::broadcast;
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

use crate::{MessageDestination, MessageType, MpcParty, MsgId, PartyIndex};
use crate::{
    ProtocolMsg,
    delivery::{Incoming, Outgoing},
};

use super::SimResult;

const DEFAULT_CAPACITY: usize = 500;

/// Simulated async network
pub struct Network<M> {
    channel: broadcast::Sender<Outgoing<Incoming<M>>>,
    next_party_idx: PartyIndex,
    next_msg_id: Arc<NextMessageId>,
}

impl<M> Network<M>
where
    M: ProtocolMsg + Clone + Send + Unpin + 'static,
{
    /// Instantiates a new simulation
    pub fn new() -> Self {
        Self::with_capacity(500)
    }

    /// Instantiates a new simulation with given capacity
    ///
    /// `Simulation` stores internally all sent messages. Capacity limits size of the internal buffer.
    /// Because of that you might run into error if you choose too small capacity. Choose capacity
    /// that can fit all the messages sent by all the parties during entire protocol lifetime.
    ///
    /// Default capacity is 500 (i.e. if you call `Simulation::new()`)
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            channel: broadcast::channel(capacity).0,
            next_party_idx: 0,
            next_msg_id: Default::default(),
        }
    }

    /// Adds new party to the network
    pub fn add_party(&mut self) -> MpcParty<M, MockedDelivery<M>> {
        MpcParty::connected(self.connect_new_party())
    }

    /// Connects new party to the network
    ///
    /// Similar to [`.add_party()`](Self::add_party) but returns `MockedDelivery<M>` instead of
    /// `MpcParty<M, MockedDelivery<M>>`
    pub fn connect_new_party(&mut self) -> MockedDelivery<M> {
        let local_party_idx = self.next_party_idx;
        self.next_party_idx += 1;

        MockedDelivery::new(
            MockedIncoming {
                local_party_idx,
                receiver: BroadcastStream::new(self.channel.subscribe()),
            },
            MockedOutgoing {
                local_party_idx,
                sender: self.channel.clone(),
                next_msg_id: self.next_msg_id.clone(),
            },
        )
    }
}

impl<M> Default for Network<M>
where
    M: ProtocolMsg + Clone + Send + Unpin + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Mocked networking
pub type MockedDelivery<M> = crate::mpc::Halves<MockedIncoming<M>, MockedOutgoing<M>>;

/// Delivery error
#[derive(Debug, thiserror::Error)]
pub enum MockedDeliveryError {
    /// Error occurred when sending a message
    #[error(transparent)]
    Recv(BroadcastStreamRecvError),
    /// Error occurred when receiving a message
    #[error(transparent)]
    Send(broadcast::error::SendError<()>),
}

/// Incoming channel of mocked network
pub struct MockedIncoming<M> {
    local_party_idx: PartyIndex,
    receiver: BroadcastStream<Outgoing<Incoming<M>>>,
}

impl<M> Stream for MockedIncoming<M>
where
    M: Clone + Send + 'static,
{
    type Item = Result<Incoming<M>, MockedDeliveryError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let msg = match ready!(Pin::new(&mut self.receiver).poll_next(cx)) {
                Some(Ok(m)) => m,
                Some(Err(e)) => return Poll::Ready(Some(Err(MockedDeliveryError::Recv(e)))),
                None => return Poll::Ready(None),
            };
            if msg.recipient.is_p2p()
                && msg.recipient != MessageDestination::OneParty(self.local_party_idx)
            {
                continue;
            }
            return Poll::Ready(Some(Ok(msg.msg)));
        }
    }
}

/// Outgoing channel of mocked network
pub struct MockedOutgoing<M> {
    local_party_idx: PartyIndex,
    sender: broadcast::Sender<Outgoing<Incoming<M>>>,
    next_msg_id: Arc<NextMessageId>,
}

impl<M> Sink<Outgoing<M>> for MockedOutgoing<M> {
    type Error = MockedDeliveryError;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, msg: Outgoing<M>) -> Result<(), Self::Error> {
        let msg_type = match msg.recipient {
            MessageDestination::AllParties { reliable } => MessageType::Broadcast { reliable },
            MessageDestination::OneParty(_) => MessageType::P2P,
        };
        self.sender
            .send(msg.map(|m| Incoming {
                id: self.next_msg_id.next(),
                sender: self.local_party_idx,
                msg_type,
                msg: m,
            }))
            .map_err(|_| MockedDeliveryError::Send(broadcast::error::SendError(())))?;
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

#[derive(Default)]
struct NextMessageId(AtomicU64);

impl NextMessageId {
    pub fn next(&self) -> MsgId {
        self.0.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
    }
}

/// Simulates execution of the protocol
///
/// Takes amount of participants, and a function that carries out the protocol for
/// one party. The function takes as input: index of the party, and [`MpcParty`]
/// that can be used to communicate with others.
///
/// ## Example
/// ```rust,no_run
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// use round_based::{Mpc, PartyIndex};
///
/// # type Result<T, E = ()> = std::result::Result<T, E>;
/// # type Randomness = [u8; 32];
/// # #[derive(round_based::ProtocolMsg, Clone)]
/// # enum Msg {}
/// // Any MPC protocol you want to test
/// pub async fn protocol_of_random_generation<M>(
///     party: M,
///     i: PartyIndex,
///     n: u16
/// ) -> Result<Randomness>
/// where
///     M: Mpc<Msg = Msg>
/// {
///     // ...
/// # todo!()
/// }
///
/// let n = 3;
///
/// let output = round_based::sim::async_env::run(
///     n,
///     |i, party| protocol_of_random_generation(party, i, n),
/// )
/// .await
/// // unwrap `Result`s
/// .expect_ok()
/// // check that all parties produced the same response
/// .expect_eq();
///
/// println!("Output randomness: {}", hex::encode(output));
/// # }  
/// ```
pub async fn run<M, F>(
    n: u16,
    party_start: impl FnMut(u16, MpcParty<M, MockedDelivery<M>>) -> F,
) -> SimResult<F::Output>
where
    M: ProtocolMsg + Clone + Send + Unpin + 'static,
    F: Future,
{
    run_with_capacity(DEFAULT_CAPACITY, n, party_start).await
}

/// Simulates execution of the protocol
///
/// Same as [`run`] but also takes a capacity of internal buffer to be used
/// within simulated network. Size of internal buffer should fit total amount of the
/// messages sent by all participants during the whole protocol execution.
pub async fn run_with_capacity<M, F>(
    capacity: usize,
    n: u16,
    mut party_start: impl FnMut(u16, MpcParty<M, MockedDelivery<M>>) -> F,
) -> SimResult<F::Output>
where
    M: ProtocolMsg + Clone + Send + Unpin + 'static,
    F: Future,
{
    run_with_capacity_and_setup(
        capacity,
        core::iter::repeat_n((), n.into()),
        |i, party, ()| party_start(i, party),
    )
    .await
}

/// Simulates execution of the protocol
///
/// Similar to [`run`], but allows some setup to be provided to the protocol execution
/// function.
///
/// Simulation will have as many parties as `setups` iterator yields
///
/// ## Example
/// ```rust,no_run
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// use round_based::{Mpc, PartyIndex};
///
/// # type Result<T, E = ()> = std::result::Result<T, E>;
/// # type Randomness = [u8; 32];
/// # #[derive(round_based::ProtocolMsg, Clone)]
/// # enum Msg {}
/// // Any MPC protocol you want to test
/// pub async fn protocol_of_random_generation<M>(
///     rng: impl rand::RngCore,
///     party: M,
///     i: PartyIndex,
///     n: u16
/// ) -> Result<Randomness>
/// where
///     M: Mpc<Msg = Msg>
/// {
///     // ...
/// # todo!()
/// }
///
/// let mut rng = rand_dev::DevRng::new();
/// let n = 3;
/// let output = round_based::sim::async_env::run_with_setup(
///     core::iter::repeat_with(|| rng.fork()).take(n.into()),
///     |i, party, rng| protocol_of_random_generation(rng, party, i, n),
/// )
/// .await
/// // unwrap `Result`s
/// .expect_ok()
/// // check that all parties produced the same response
/// .expect_eq();
///
/// println!("Output randomness: {}", hex::encode(output));
/// # }  
/// ```
pub async fn run_with_setup<S, M, F>(
    setups: impl IntoIterator<Item = S>,
    party_start: impl FnMut(u16, MpcParty<M, MockedDelivery<M>>, S) -> F,
) -> SimResult<F::Output>
where
    M: ProtocolMsg + Clone + Send + Unpin + 'static,
    F: Future,
{
    run_with_capacity_and_setup::<S, M, F>(DEFAULT_CAPACITY, setups, party_start).await
}

/// Simulates execution of the protocol
///
/// Same as [`run_with_setup`] but also takes a capacity of internal buffer to be used
/// within simulated network. Size of internal buffer should fit total amount of the
/// messages sent by all participants during the whole protocol execution.
pub async fn run_with_capacity_and_setup<S, M, F>(
    capacity: usize,
    setups: impl IntoIterator<Item = S>,
    mut party_start: impl FnMut(u16, MpcParty<M, MockedDelivery<M>>, S) -> F,
) -> SimResult<F::Output>
where
    M: ProtocolMsg + Clone + Send + Unpin + 'static,
    F: Future,
{
    let mut network = Network::<M>::with_capacity(capacity);

    let mut output = alloc::vec![];
    for (setup, i) in setups.into_iter().zip(0u16..) {
        output.push({
            let party = network.add_party();
            party_start(i, party, setup)
        });
    }

    let result = futures_util::future::join_all(output).await;
    SimResult(result)
}
