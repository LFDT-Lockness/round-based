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
//!   We use abstractions [`Stream`] and [`Sink`] to receive and send messages.
//!
//! ## Networking
//!
//! In order to run an MPC protocol, transport layer needs to be defined. All you have to do is to
//! provide a channel which implements a stream and a sink for receiving and sending messages.
//!
//! Message delivery should meet certain criterias that differ from protocol to protocol (refer to
//! the documentation of the protocol you're using), but usually they are:
//!
//! * Messages should be authenticated \
//!   Each message should be signed with identity key of the sender. This implies having Public Key
//!   Infrastructure.
//! * P2P messages should be encrypted \
//!   Only recipient should be able to learn the content of p2p message
//! * Broadcast channel should be reliable \
//!   Some protocols may require broadcast channel to be reliable. Simply saying, when party receives a
//!   broadcast message over reliable channel it should be ensured that everybody else received the same
//!   message.
//!
//! ## Features
//!
//! * `sim` enables protocol execution simulation, see [`sim`] module
//!   * `sim-async` enables protocol execution simulation with tokio runtime, see [`sim::async_env`]
//!     module
//! * `state-machine` provides ability to carry out the protocol, defined as async function, via Sync
//!   API, see [`state_machine`] module
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

#[doc(no_inline)]
pub use futures_util::{Sink, SinkExt, Stream, StreamExt};

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
