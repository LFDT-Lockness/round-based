//! Multiparty protocol simulation
//!
//! [`Simulation`] is an essential developer tool for testing the multiparty protocol locally.
//! It covers most of the boilerplate by mocking networking.
//!
//! ## Example
//!
//! ```rust
//! use round_based::{Mpc, PartyIndex};
//! use round_based::simulation::Simulation;
//! use futures::future::try_join_all;
//!
//! # type Result<T, E = ()> = std::result::Result<T, E>;
//! # type Randomness = [u8; 32];
//! # type Msg = ();
//! // Any MPC protocol you want to test
//! pub async fn protocol_of_random_generation<M>(party: M, i: PartyIndex, n: u16) -> Result<Randomness>
//! where M: Mpc<ProtocolMessage = Msg>
//! {
//!     // ...
//! # todo!()
//! }
//!
//! async fn test_randomness_generation() {
//!     let n = 3;
//!
//!     let mut simulation = Simulation::<Msg>::new();
//!     let mut outputs = vec![];
//!     for i in 0..n {
//!         let party = simulation.add_party();
//!         outputs.push(protocol_of_random_generation(party, i, n));
//!     }   
//!
//!     // Waits each party to complete the protocol
//!     let outputs = try_join_all(outputs).await.expect("protocol wasn't completed successfully");
//!     // Asserts that all parties output the same randomness
//!     for output in outputs.iter().skip(1) {
//!         assert_eq!(&outputs[0], output);
//!     }  
//! }
//! ```

mod sim_async;
#[cfg(feature = "state-machine")]
mod sim_sync;

pub use sim_async::*;
#[cfg(feature = "state-machine")]
pub use sim_sync::*;
