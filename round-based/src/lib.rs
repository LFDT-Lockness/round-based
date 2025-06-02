//! ![License: MIT](https://img.shields.io/crates/l/round-based.svg)
//! [![Docs](https://docs.rs/round-based/badge.svg)](https://docs.rs/round-based)
//! [![Crates io](https://img.shields.io/crates/v/round-based.svg)](https://crates.io/crates/round-based)
//! [![Discord](https://img.shields.io/discord/905194001349627914?logo=discord&logoColor=ffffff&label=Discord)](https://discordapp.com/channels/905194001349627914/1285268686147424388)
//!
//! An MPC framework that unifies and simplifies the way of developing and working with
//! multiparty protocols (e.g. threshold signing, random beacons, etc.).
//!
//! ## Goals
//!
//! * Async friendly \
//!   Async is the most simple and efficient way of doing networking in Rust
//! * Simple, configurable \
//!   Protocol can be carried out in a few lines of code: check out examples.
//! * Independent of networking layer \
//!   You may define your own networking layer and you don't need to change anything
//!   in protocol implementation: it's agnostic of networking by default! So you can
//!   use central delivery server, distributed redis nodes, postgres database,
//!   p2p channels, or a public blockchain, or whatever else fits your needs.
//!
//! ## Example
//! MPC protocol execution typically looks like this:
//!
//! ```rust
//! # #[derive(round_based::ProtocolMsg)]
//! # enum KeygenMsg {}
//! # struct KeyShare;
//! # struct Error;
//! # type Result<T> = std::result::Result<T, Error>;
//! # async fn doc() -> Result<()> {
//! // protocol to be executed, takes MPC engine `M`, index of party `i`,
//! // and number of participants `n`
//! async fn keygen<M>(mpc: M, i: u16, n: u16) -> Result<KeyShare>
//! where
//!     M: round_based::Mpc<Msg = KeygenMsg>
//! {
//!     // ...
//! # unimplemented!()
//! }
//! // establishes network connection(s) to other parties so they may communicate
//! async fn connect() ->
//!     impl futures::Stream<Item = Result<round_based::Incoming<KeygenMsg>>>
//!         + futures::Sink<round_based::Outgoing<KeygenMsg>, Error = Error>
//!         + Unpin
//! {
//!     // ...
//! # round_based::_docs::fake_delivery()
//! }
//! let delivery = connect().await;
//!
//! // constructs an MPC engine, which, primarily, is used to communicate with
//! // other parties
//! let mpc = round_based::mpc::connected(delivery);
//!
//! # let (i, n) = (1, 3);
//! // execute the protocol
//! let keyshare = keygen(mpc, i, n).await?;
//! # Ok(()) }
//! ```
//!
//! ## Networking
//!
//! In order to run an MPC protocol, transport layer needs to be defined. All you have to do is to
//! provide a channel which implements a stream and a sink for receiving and sending messages.
//!
//! ```rust,no_run
//! # #[derive(round_based::ProtocolMsg)]
//! # enum Msg {}
//! # struct Error;
//! # type Result<T> = std::result::Result<T, Error>;
//! # async fn doc() -> Result<()> {
//! async fn connect() ->
//!     impl futures::Stream<Item = Result<round_based::Incoming<Msg>>>
//!         + futures::Sink<round_based::Outgoing<Msg>, Error = Error>
//!         + Unpin
//! {
//!     // ...
//! # round_based::_docs::fake_delivery()
//! }
//!
//! let delivery = connect().await;
//! let party = round_based::mpc::connected(delivery);
//!
//! // run the protocol
//! # Ok(()) }
//! ```
//!
//! In order to guarantee the protocol security, it may require:
//!
//! * Message Authentication \
//!   Guarantees message source and integrity. If protocol requires it, make sure
//!   message was sent by claimed sender and that it hasn't been tampered with. \
//!   This is typically achieved either through public-key cryptography (e.g.,
//!   signing with a private key) or through symmetric mechanisms like MACs (e.g.,
//!   HMAC) or authenticated encryption (AEAD) in point-to-point scenarios.
//! * Message Privacy \
//!   When a p2p message is sent, only recipient shall be able to read the content. \
//!   It can be achieved by using symmetric or asymmetric encryption, encryption methods
//!   come with their own trade-offs (e.g. simplicity vs forward secrecy).
//! * Reliable Broadcast \
//!   When party receives a reliable broadcast message it shall be ensured that
//!   everybody else received the same message. \
//!   Our library provides [`echo_broadcast`] support out-of-box that enforces broadcast
//!   reliability by adding an extra communication round per each round that requires
//!   reliable broadcast. \
//!   More advanced techniques implement [Byzantine fault](https://en.wikipedia.org/wiki/Byzantine_fault)
//!   tolerant broadcast
//!
//! ## Developing MPC protocol with `round_based`
//! We plan to write a book guiding through MPC protocol development process, but
//! while it's not done, you may refer to [random beacon example](https://github.com/LFDT-Lockness/round-based/blob/m/examples/random-generation-protocol/src/lib.rs)
//! and our well-documented API.
//!
//! ## Features
//!
//! * `sim` enables protocol execution simulation, see [`sim`] module
//!   * `sim-async` enables protocol execution simulation with tokio runtime, see [`sim::async_env`]
//!     module
//! * `state-machine` provides ability to carry out the protocol, defined as async function, via Sync
//!   API, see [`state_machine`] module
//! * `echo-broadcast` adds [`echo_broadcast`] support
//! * `derive` is needed to use [`ProtocolMsg`](macro@ProtocolMsg) proc macro
//! * `runtime-tokio` enables [tokio]-specific implementation of [async runtime](mpc::party::runtime)
//!
//! ## Join us in Discord!
//! Feel free to reach out to us [in Discord](https://discordapp.com/channels/905194001349627914/1285268686147424388)!

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg, doc_cfg_hide))]
#![warn(unused_crate_dependencies, missing_docs)]
#![allow(async_fn_in_trait)]
#![no_std]

extern crate alloc;

/// Fixes false-positive of `unused_crate_dependencies` lint that only occur in the tests
#[cfg(test)]
mod false_positives {
    use anyhow as _;
    use futures as _;
    use trybuild as _;

    use {hex as _, rand as _, rand_dev as _};
}

mod delivery;
#[cfg(feature = "echo-broadcast")]
pub mod echo_broadcast;
pub mod mpc;
pub mod round;
#[cfg(feature = "state-machine")]
pub mod state_machine;

#[cfg(feature = "sim")]
pub mod sim;

pub use self::delivery::*;
pub use self::mpc::MpcParty;
#[doc(no_inline)]
pub use self::mpc::{Mpc, MpcExecution, ProtocolMsg, RoundMsg};

#[doc(hidden)]
pub mod _docs;

/// Derives [`ProtocolMsg`] and [`RoundMsg`] traits
///
/// See [`ProtocolMsg`] docs for more details
#[cfg(feature = "derive")]
pub use round_based_derive::ProtocolMsg;
