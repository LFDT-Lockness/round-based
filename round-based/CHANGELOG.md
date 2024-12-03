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

Migration guidelines:
* Replace `dev` feature with `sim`
* Instead of using `round_based::simulation::Simulation` from previous version, use
  `round_based::simulation::{run, run_with_setup}`
* Take advantage of `SimResult::{expect_ok, expect_eq}` to reduce amount of the code
  in your tests

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
