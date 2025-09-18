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
  You may define your own networking layer and you don't need to change anything
  in protocol implementation: it's agnostic of networking by default! So you can
  use central delivery server, distributed redis nodes, postgres database,
  p2p channels, or a public blockchain, or whatever else fits your needs.

## Example
MPC protocol execution typically looks like this:

```rust
// protocol to be executed, takes MPC engine `M`, index of party `i`,
// and number of participants `n`
async fn keygen<M>(mpc: M, i: u16, n: u16) -> Result<KeyShare>
where
    M: round_based::Mpc<Msg = KeygenMsg>
{
    // ...
}
// establishes network connection(s) to other parties so they may communicate
async fn connect() ->
    impl futures::Stream<Item = Result<round_based::Incoming<KeygenMsg>>>
        + futures::Sink<round_based::Outgoing<KeygenMsg>, Error = Error>
        + Unpin
{
    // ...
}
let delivery = connect().await;

// constructs an MPC engine, which, primarily, is used to communicate with
// other parties
let mpc = round_based::mpc::connected(delivery);

// execute the protocol
let keyshare = keygen(mpc, i, n).await?;
```

## Networking

In order to run an MPC protocol, transport layer needs to be defined. All you have to do is to
provide a channel which implements a stream and a sink for receiving and sending messages.

```rust
async fn connect() ->
    impl futures::Stream<Item = Result<round_based::Incoming<Msg>>>
        + futures::Sink<round_based::Outgoing<Msg>, Error = Error>
        + Unpin
{
    // ...
}

let delivery = connect().await;
let party = round_based::mpc::connected(delivery);

// run the protocol
```

In order to guarantee the protocol security, it may require:

* Message Authentication \
  Guarantees message source and integrity. If protocol requires it, make sure
  message was sent by claimed sender and that it hasn't been tampered with. \
  This is typically achieved either through public-key cryptography (e.g.,
  signing with a private key) or through symmetric mechanisms like MACs (e.g.,
  HMAC) or authenticated encryption (AEAD) in point-to-point scenarios.
* Message Privacy \
  When a p2p message is sent, only recipient shall be able to read the content. \
  It can be achieved by using symmetric or asymmetric encryption, encryption methods
  come with their own trade-offs (e.g. simplicity vs forward secrecy).
* Reliable Broadcast \
  When party receives a reliable broadcast message it shall be ensured that
  everybody else received the same message. \
  Our library provides `echo_broadcast` support out-of-box that enforces broadcast
  reliability by adding an extra communication round per each round that requires
  reliable broadcast. \
  More advanced techniques implement [Byzantine fault](https://en.wikipedia.org/wiki/Byzantine_fault)
  tolerant broadcast

## Developing MPC protocol with `round_based`
We plan to write a book guiding through MPC protocol development process, but
while it's not done, you may refer to [random beacon example](https://github.com/LFDT-Lockness/round-based/blob/m/examples/random-generation-protocol/src/lib.rs)
and our well-documented API.

## Features

* `sim` enables protocol execution simulation, see `sim` module
  * `sim-async` enables protocol execution simulation with tokio runtime, see `sim::async_env`
    module
* `state-machine` provides ability to carry out the protocol, defined as async function, via Sync
  API, see `state_machine` module
* `echo-broadcast` adds `echo_broadcast` support
* `derive` is needed to use `ProtocolMsg` proc macro
* `runtime-tokio` enables tokio-specific implementation of async runtime

## Join us in Discord!
Feel free to reach out to us [in Discord](https://discordapp.com/channels/905194001349627914/1285268686147424388)!
