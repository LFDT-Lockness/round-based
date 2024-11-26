//! Multiparty protocol simulation
//!
//! Simulator is an essential developer tool for testing the multiparty protocol locally.
//! It covers most of the boilerplate by mocking networking.
//!
//! The entry point is either [`run`] or [`run_with_setup`] functions. They take a protocol
//! defined as an async function, provide simulated networking, carry out the simulation,
//! and return the result.
//!
//! If you need more control over execution, you can use [`Network`] to simulate the networking
//! and carry out the protocol manually.
//!
//! When `state-machine` feature is enabled, [`SimulationSync`] is available which can carry out
//! protocols defined as a state machine.
//!
//! ## Example
//! ```rust,no_run
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! use round_based::{Mpc, PartyIndex};
//!
//! # type Result<T, E = ()> = std::result::Result<T, E>;
//! # type Randomness = [u8; 32];
//! # type Msg = ();
//! // Any MPC protocol you want to test
//! pub async fn protocol_of_random_generation<M>(
//!     party: M,
//!     i: PartyIndex,
//!     n: u16
//! ) -> Result<Randomness>
//! where
//!     M: Mpc<ProtocolMessage = Msg>
//! {
//!     // ...
//! # todo!()
//! }
//!
//! let n = 3;
//!
//! let output = round_based::simulation::run(
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

mod sim_async;
#[cfg(feature = "state-machine")]
mod sim_sync;

pub use sim_async::*;
#[cfg(feature = "state-machine")]
pub use sim_sync::*;

/// Result of the simulation
pub struct SimResult<T>(pub alloc::vec::Vec<T>);

impl<T, E> SimResult<Result<T, E>>
where
    E: core::fmt::Debug,
{
    /// Unwraps `Result<T, E>` produced by each party
    ///
    /// Panics if at least one of the parties returned `Err(_)`. In this case,
    /// a verbose error message will shown specifying which of the parties returned
    /// an error.
    pub fn expect_ok(self) -> SimResult<T> {
        let mut oks = alloc::vec::Vec::with_capacity(self.0.len());
        let mut errs = alloc::vec::Vec::with_capacity(self.0.len());

        for (res, i) in self.0.into_iter().zip(0u16..) {
            match res {
                Ok(res) => oks.push(res),
                Err(res) => errs.push((i, res)),
            }
        }

        if !errs.is_empty() {
            let mut msg = alloc::format!(
                "Simulation output didn't match expectations.\n\
                Expected: all parties succeed\n\
                Actual  : {success} parties succeeded, {failed} parties returned an error\n\
                Failures:\n",
                success = oks.len(),
                failed = errs.len(),
            );

            for (i, err) in errs {
                msg += &alloc::format!("- Party {i}: {err:?}\n");
            }

            panic!("{msg}");
        }

        SimResult(oks)
    }
}

impl<T> SimResult<T>
where
    T: PartialEq + core::fmt::Debug,
{
    /// Checks that outputs of all parties are equally the same
    ///
    /// Returns the output on success (all the outputs are checked to be the same), otherwise
    /// panics with a verbose error message.
    ///
    /// Panics if simulation contained zero parties.
    pub fn expect_eq(mut self) -> T {
        let Some(first) = self.0.first() else {
            panic!("simulation contained zero parties");
        };

        if !self.0[1..].iter().all(|i| i == first) {
            let mut msg = alloc::string::String::from(
                "Simulation output didn't match expectations.\n\
                Expected: all parties return the same output\n\
                Actual  : some of the parties returned a different output\n\
                Outputs :\n",
            );

            for (i, res) in self.0.iter().enumerate() {
                msg += &alloc::format!("- Party {i}: {res:?}");
            }

            panic!("{msg}")
        }

        self.0
            .pop()
            .expect("we checked that the list contains at least one element")
    }
}

impl<T> SimResult<T> {
    /// Deconstructs the simulation result returning inner list of results
    pub fn into_vec(self) -> alloc::vec::Vec<T> {
        self.0
    }
}

impl<T> From<alloc::vec::Vec<T>> for SimResult<T> {
    fn from(list: alloc::vec::Vec<T>) -> Self {
        Self(list)
    }
}

impl<T> From<SimResult<T>> for alloc::vec::Vec<T> {
    fn from(res: SimResult<T>) -> Self {
        res.0
    }
}
