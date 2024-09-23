![License: MIT](https://img.shields.io/crates/l/round-based.svg)
[![Docs](https://docs.rs/round-based/badge.svg)](https://docs.rs/round-based)
[![Crates io](https://img.shields.io/crates/v/round-based.svg)](https://crates.io/crates/round-based)
[![Discord](https://img.shields.io/discord/905194001349627914?logo=discord&logoColor=ffffff&label=Discord)](https://discordapp.com/channels/905194001349627914/1285268686147424388)

An MPC framework that unifies and simplifies the way of developing and working with
multiparty protocols (e.g. threshold signing, random beacons, etc.).

## Goals

* Async friendly \
  Async is the most simple and efficient way of doing networking in Rust
* Simple, configurable \
  Protocol can be carried out in a few lines of code: check out examples.
* Independent of networking layer \
  We use abstractions `Stream` and `Sink` to receive and send messages.

## Networking

In order to run an MPC protocol, transport layer needs to be defined. All you have to do is to
implement `Delivery` trait which is basically a stream and a sink for receiving and sending messages.

Message delivery should meet certain criterias that differ from protocol to protocol (refer to
the documentation of the protocol you're using), but usually they are:

* Messages should be authenticated \
  Each message should be signed with identity key of the sender. This implies having Public Key
  Infrastructure.
* P2P messages should be encrypted \
  Only recipient should be able to learn the content of p2p message
* Broadcast channel should be reliable \
  Some protocols may require broadcast channel to be reliable. Simply saying, when party receives a
  broadcast message over reliable channel it should be ensured that everybody else received the same
  message.

## Features

* `dev` enables development tools such as protocol simulation
* `runtime-tokio` enables tokio-specific implementation of async runtime

## Join us in Discord!
Feel free to reach out to us in Discord!
