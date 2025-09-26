## v0.5.0
**For protocol devs:** if you developed an MPC protocol on `round_based`, here's a list of relevant API changes:
* `round_based::rounds_router` is removed \
  Previously, protocol implementations used to use `round_based::rounds_router::RoundsRouter` to
  register rounds of the protocol. This was a typical workflow:
  ```rust
  // Construct a router builder
  let mut rounds = RoundsRouter::<Msg>::builder();
  // Register rounds
  let round1 = rounds.add_round(RoundInput::<CommitMsg>::broadcast(i, n));
  let round2 = rounds.add_round(RoundInput::<DecommitMsg>::broadcast(i, n));
  // Complete the router
  let mut rounds = rounds.listen(incoming);

  // ... run the protocol

  // use the router to receive messages in the round:
  let commitments = rounds
      .complete(round1)
      .await
      .map_err(Error::Round1Receive)?;
  ```
  With the new API, all of this is integrated into `Mpc` trait. Given `mut mpc: M` provided as an input
  to the protocol (where `M: Mpc`), this is how typical setup looks like:
  ```rust
  // Register rounds
  let round1 = mpc.add_round(round_based::round::reliable_broadcast::<CommitMsg>(i, n));
  let round2 = mpc.add_round(round_based::round::broadcast::<DecommitMsg>(i, n));
  // Complete the router
  let mut mpc = mpc.finish_setup();

  // ... run the protocol

  // use `mpc` to send messages in the round:
  mpc.reliably_broadcast(Msg::CommitMsg(CommitMsg { commitment }))
      .await
      .map_err(Error::Round1Send)?;

  // use `mpc` to receive messages in the round:
  let commitments = mpc.complete(round1).await.map_err(Error::Round1Receive)?;
  ```
* We now distinguish reliable broadcast messages:
  * Message is reliably broadcasted if on reception the receiver knows that all other honest
    parties received exactly the same message.
  * For incoming messages, `round_based::Incoming` has field `msg_type` which contains a flag
    `reliable: bool` for broadcast messages that indicates whether message was verified to be
    reliably broadcasted
  * For outgoing messages, `round_based::Outgoing` has field `recipient` which contains a flag
    `reliable: bool` for broadcast messages that indicates whether message has to be reliably
    broadcasted
  * Protocols can use `round_based::round::reliable_broadcast(i, n)` to create a reliable broadcast
    round. If received message wasn't reliably broadcasted (`reliable` flag is set to `false`),
    round fails with an error.
  * Delivery implementations must take into account the new flag. If protocol wants to send
    a reliable broadcast message, but delivery layer doesn't support this, an error must be
    returned.
  * For any delivery implementation that does not support reliable broadcast, we provide a
    `round_based::echo_broadcast` primitive that implements cryptographic reliable broadcast
    for any protocol. Check out this module docs if you need more info.
* **TL;DR** of new API:
  * Use `Mpc::add_round` to register rounds of the protocol
  * Once all rounds are registered, use `Mpc::finish_setup` that returns `MpcExecution`
  * To send messages, use:
    * `MpcExecution::{send, send_p2p, send_broadcast, reliably_broadcast}` to send one message
    * `MpcExecution::send_many` to send many messages at once
  * To receive messages from a round `r`, use `MpcExecution::complete(r)`
  * Use `MpcExecution::yield_now` to temporarily return execution to async runtime. Use it to
    break a long execution into smaller pieces.
  * `round_based::{Incoming, Outgoing}` now explicitly specify if message was (needs to be)
    reliably broadcasted.
* Check out changes in our example protocol for randomness generation to see changes that were
  needed to upgrade to latest API: [see diff](https://github.com/LFDT-Lockness/round-based/compare/bc9dcd8..v0.4.1#diff-fc41689d56219af09956ddb6d614b77f6974566ccc616356befa8cdece36bf19)

**For protocol users:** if you use a protocol that is built on top of `round_based`, here's a list
of relevant changes in API:
* `MpcParty` construction flow:
  * Previously, you used to construct a `MpcParty` by providing `Delivery` implementation via
    `MpcParty::connected`, then it was provided to the protocol as an input.
  * Now, `Delivery` trait was removed. Instead you need to construct a `channel` that implements both
    `Stream<Result<Incoming<M>, Error>>` and `Sink<Outgoing<M>, Error = Error>`. Then construct an
    `MpcParty` by calling `round_based::mpc::connected(channel)` (if you have separate stream and sink
    channels, use `round_based::mpc::connected_halves(stream, sink)`).
  * Provide `MpcParty` to the protocol as an input.
* Reliable broadcast
  * Message is reliably broadcasted if on reception the receiver knows that all other honest parties
    received exactly the same message.
  * Some protocols require messages at some rounds to be reliably broadcasted
  * If they do, it will be indicated in `round_based::Outgoing` in field `msg_type` that now has a
    flag `reliable: bool` which indicates if message has to be reliably broadcasted.
  * Delivery implementation that doesn't support reliable broadcast must return an error if protocol
    tries to send a reliable broadcast message
  * Delivery implementation must set `reliable` flag to `true` for incoming message only if it
    was cryptographically (or through other trust assumptions) checked that message was reliably
    broacasted
  * For any delivery implementation that does not support reliable broadcast, we provide a
    `round_based::echo_broadcast` primitive that implements cryptographic reliable broadcast
    for any protocol. Check out this module docs if you need more info.

## v0.4.1
* Add methods to MpcParty to change its components [#15]

[#15]: https://github.com/LFDT-Lockness/round-based/pull/15

## v0.4.0
* BREAKING: Improve ergonomics of protocol simulation, which is used for writing tests [#14]
  * Remove `dev` feature, it's replaced with `sim` and `sim-async`
  * `round_based::simulation` module is renamed into `round_based::sim`
  * `round_based::simulation::Simulation` is renamed and moved to `round_based::sim::async_env::Network`
  * Other async simulated network related types are moved to `round_based::sim::async_env`
  * Added convenient `round_based::sim::{run, run_with_setup}` which make simulation very ergonomic
  * Simulation outputs `round_based::sim::SimResult`, which has convenient most-common methods:
    * `.expect_ok()` that unwraps all results, and if any party returned an error, panics with a verbose
      error message
    * `.expect_eq()` that checks that all outputs are equally the same
  * When `sim-async` feature is enabled, you can use `round_based::sim::async_env::{run, run_with_setup, ...}`,
    but typically you don't want to use them
  * `round_based::simulation::SimulationSync` has been renamed to `round_based::sim::Simulation`
* Use `core::error::Error` trait which is now always implemented for all errors regardless whether `std` feature
  is enabled or not [#14]
  * Update `thiserror` dependency to v2
  * BREAKING: remove `std` feature, as the crate is fully no_std now

Migration guidelines:
* Replace `dev` feature with `sim`
* Instead of using `round_based::simulation::Simulation` from previous version, use
  `round_based::simulation::{run, run_with_setup}`
* Take advantage of `SimResult::{expect_ok, expect_eq}` to reduce amount of the code
  in your tests
* Remove `std` feature, if it was explicitly enabled

Other than simulation, there are no breaking changes in this release.

[#14]: https://github.com/LFDT-Lockness/round-based/pull/14

## v0.3.2
* Update links in crate settings, update readme [#11]

[#11]: https://github.com/LFDT-Lockness/round-based/pull/11

## v0.3.1
* Add `rounds_router::simple_store::RoundMsgs::into_iter_including_me()` [#9]

[#9]: https://github.com/LFDT-Lockness/round-based/pull/9

## v0.3.0
* Add no_std and wasm support [#6]
* Add state machine wrapper that provides sync API to carry out the protocol defined as async function [#7]

[#6]: https://github.com/LFDT-Lockness/round-based/pull/6
[#7]: https://github.com/LFDT-Lockness/round-based/pull/7

## v0.2.2

* fix: correct handling of stores that need no messages in RoundsRouter [#4]

[#4]: https://github.com/LFDT-Lockness/round-based/pull/4

## v0.2.1

Changes prior this version weren't documented
