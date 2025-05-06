//! Multiparty protocol simulation
//!
//! Simulator is an essential developer tool for testing the multiparty protocol locally.
//! It covers most of the boilerplate of emulating MPC protocol execution.
//!
//! The entry point is either [`run`] or [`run_with_setup`] functions. They take a protocol
//! defined as an async function, provide simulated networking, carry out the simulation,
//! and return the result.
//!
//! If you need more control over execution, you can use [`Simulation`]. For instance, it allows
//! creating a simulation that has parties defined by different functions, which is helpful, for
//! instance, in simulation in presence of an adversary (e.g. one set of parties can be defined
//! with a regular function/protocol implementation, when the other set of parties may be defined
//! by other function which emulates adversary behavior).
//!
//! ## Limitations
//! [`Simulation`] works by converting each party (defined as an async function) into the
//! [state machine](crate::state_machine). That should work without problems in most cases, providing
//! better UX, without requiring an async runtime (simulation is entirely sync).
//!
//! However, a protocol wrapped into a state machine cannot poll any futures except provided within
//! [`MpcParty`](crate::MpcParty) (so it can only await on sending/receiving messages and yielding).
//! For instance, if the protocol implementation makes use of tokio timers, it will result into an
//! execution error.
//!
//! In general, we do not recommend awaiting on the futures that aren't provided by `MpcParty` in
//! the MPC protocol implementation, to keep the protocol implementation runtime-agnostic.
//!
//! If you do really need to make use of unsupported futures, you can use [`async_env`] instead,
//! which provides a simulation on tokio runtime, but has its own limitations.
//!
//! ## Example
//! ```rust,no_run
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
//! let output = round_based::sim::run(
//!     n,
//!     |i, party| protocol_of_random_generation(party, i, n),
//! )
//! .unwrap()
//! // unwrap `Result`s
//! .expect_ok()
//! // check that all parties produced the same response
//! .expect_eq();
//!
//! println!("Output randomness: {}", hex::encode(output));
//! ```

use alloc::{boxed::Box, collections::VecDeque, string::ToString, vec::Vec};
use core::future::Future;

use crate::{
    state_machine::ProceedResult, Incoming, MessageDestination, MessageType, Outgoing, ProtocolMsg,
};

#[cfg(feature = "sim-async")]
pub mod async_env;

/// Result of the simulation
pub struct SimResult<T>(pub Vec<T>);

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
        let mut oks = Vec::with_capacity(self.0.len());
        let mut errs = Vec::with_capacity(self.0.len());

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

            let mut clusters: Vec<(&T, Vec<usize>)> = Vec::new();
            for (i, value) in self.0.iter().enumerate() {
                match clusters
                    .iter_mut()
                    .find(|(cluster_value, _)| *cluster_value == value)
                    .map(|(_, indexes)| indexes)
                {
                    Some(indexes) => indexes.push(i),
                    None => clusters.push((value, alloc::vec![i])),
                }
            }

            for (value, parties) in &clusters {
                if parties.len() == 1 {
                    msg += "- Party ";
                } else {
                    msg += "- Parties "
                }

                for (i, is_first) in parties
                    .iter()
                    .zip(core::iter::once(true).chain(core::iter::repeat(false)))
                {
                    if !is_first {
                        msg += ", "
                    }
                    msg += &i.to_string();
                }

                msg += &alloc::format!(": {value:?}\n");
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
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T> IntoIterator for SimResult<T> {
    type Item = T;
    type IntoIter = alloc::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T> core::ops::Deref for SimResult<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> From<Vec<T>> for SimResult<T> {
    fn from(list: Vec<T>) -> Self {
        Self(list)
    }
}

impl<T> From<SimResult<T>> for Vec<T> {
    fn from(res: SimResult<T>) -> Self {
        res.0
    }
}

/// Simulates MPC protocol with parties defined as [state machines](crate::state_machine)
pub struct Simulation<'a, O, M> {
    parties: Vec<Party<'a, O, M>>,
}

enum Party<'a, O, M> {
    Active {
        party: Box<dyn crate::state_machine::StateMachine<Output = O, Msg = M> + 'a>,
        wants_one_more_msg: bool,
    },
    Finished(O),
}

impl<'a, O, M> Simulation<'a, O, M>
where
    M: ProtocolMsg + Clone + 'static,
{
    /// Creates empty simulation containing no parties
    ///
    /// New parties can be added via [`.add_party()`](Self::add_party)
    pub fn empty() -> Self {
        Self {
            parties: Vec::new(),
        }
    }

    /// Constructs empty simulation containing no parties, with allocated memory that can fit up to `n` parties without re-allocations
    pub fn with_capacity(n: u16) -> Self {
        Self {
            parties: Vec::with_capacity(n.into()),
        }
    }

    /// Constructs a simulation with `n` parties from async function that defines the protocol
    ///
    /// Each party has index `0 <= i < n` and instantiated via provided `init` function
    ///
    /// Async function will be converted into a [state machine](crate::state_machine). Because of that,
    /// it cannot await on any futures that aren't provided by `MpcParty` (that is given as an argument
    /// to this function).
    pub fn from_async_fn<F>(
        n: u16,
        mut init: impl FnMut(u16, crate::state_machine::MpcParty<M>) -> F,
    ) -> Self
    where
        F: core::future::Future<Output = O> + 'a,
    {
        let mut sim = Self::with_capacity(n);
        for i in 0..n {
            sim.add_async_party(|party| init(i, party))
        }
        sim
    }

    /// Construct a simulation with `n` parties from `init` function that constructs state machine for each party
    ///
    /// Each party has index `0 <= i < n` and instantiated via provided `init` function
    pub fn from_fn<S>(n: u16, mut init: impl FnMut(u16) -> S) -> Self
    where
        S: crate::state_machine::StateMachine<Output = O, Msg = M> + 'a,
    {
        let mut sim = Self::with_capacity(n);
        for i in 0..n {
            sim.add_party(init(i));
        }
        sim
    }

    /// Adds new party into the protocol
    ///
    /// New party will be assigned index `i = n - 1` where `n` is amount of parties in the
    /// simulation after this party was added.
    pub fn add_party(
        &mut self,
        party: impl crate::state_machine::StateMachine<Output = O, Msg = M> + 'a,
    ) {
        self.parties.push(Party::Active {
            party: Box::new(party),
            wants_one_more_msg: false,
        })
    }

    /// Adds new party, defined as an async function, into the protocol
    ///
    /// New party will be assigned index `i = n - 1` where `n` is amount of parties in the
    /// simulation after this party was added.
    ///
    /// Async function will be converted into a [state machine](crate::state_machine). Because of that,
    /// it cannot await on any futures that aren't provided by `MpcParty` (that is given as an argument
    /// to this function).
    pub fn add_async_party<F>(&mut self, party: impl FnOnce(crate::state_machine::MpcParty<M>) -> F)
    where
        F: core::future::Future<Output = O> + 'a,
    {
        self.parties.push(Party::Active {
            party: Box::new(crate::state_machine::wrap_protocol(party)),
            wants_one_more_msg: false,
        })
    }

    /// Returns amount of parties in the simulation
    pub fn parties_amount(&self) -> usize {
        self.parties.len()
    }

    /// Carries out the simulation
    pub fn run(mut self) -> Result<SimResult<O>, SimError> {
        let mut messages_queue = MessagesQueue::new(self.parties.len());
        let mut parties_left = self.parties.len();

        while parties_left > 0 {
            'next_party: for (i, party_state) in (0..).zip(&mut self.parties) {
                'this_party: loop {
                    let Party::Active {
                        party,
                        wants_one_more_msg,
                    } = party_state
                    else {
                        continue 'next_party;
                    };

                    if *wants_one_more_msg {
                        if let Some(message) = messages_queue.recv_next_msg(i) {
                            party
                                .received_msg(message)
                                .map_err(|_| Reason::SaveIncomingMsg)?;
                            *wants_one_more_msg = false;
                        } else {
                            continue 'next_party;
                        }
                    }

                    match party.proceed() {
                        ProceedResult::SendMsg(msg) => {
                            messages_queue.send_message(i, msg)?;
                            continue 'this_party;
                        }
                        ProceedResult::NeedsOneMoreMessage => {
                            *wants_one_more_msg = true;
                            continue 'this_party;
                        }
                        ProceedResult::Output(out) => {
                            *party_state = Party::Finished(out);
                            parties_left -= 1;
                            continue 'next_party;
                        }
                        ProceedResult::Yielded => {
                            continue 'this_party;
                        }
                        ProceedResult::Error(err) => {
                            return Err(Reason::ExecutionError(err).into());
                        }
                    }
                }
            }
        }

        Ok(SimResult(
            self.parties
                .into_iter()
                .map(|party| match party {
                    Party::Active { .. } => {
                        unreachable!("there must be no active parties when `parties_left == 0`")
                    }
                    Party::Finished(out) => out,
                })
                .collect(),
        ))
    }
}

/// Error indicating that simulation failed
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct SimError(#[from] Reason);

#[derive(Debug, thiserror::Error)]
enum Reason {
    #[error("save incoming message")]
    SaveIncomingMsg,
    #[error("execution error")]
    ExecutionError(#[source] crate::state_machine::ExecutionError),
    #[error("party #{sender} tried to send a message to non existing party #{recipient}")]
    UnknownRecipient { sender: u16, recipient: u16 },
}

struct MessagesQueue<M> {
    queue: Vec<VecDeque<Incoming<M>>>,
    next_id: u64,
}

impl<M: Clone> MessagesQueue<M> {
    fn new(n: usize) -> Self {
        Self {
            queue: alloc::vec![VecDeque::new(); n],
            next_id: 0,
        }
    }

    fn send_message(&mut self, sender: u16, msg: Outgoing<M>) -> Result<(), SimError> {
        match msg.recipient {
            MessageDestination::AllParties { reliable } => {
                let mut msg_ids = self.next_id..;
                for (destination, msg_id) in (0..)
                    .zip(&mut self.queue)
                    .filter(|(recipient_index, _)| *recipient_index != sender)
                    .map(|(_, msg)| msg)
                    .zip(msg_ids.by_ref())
                {
                    destination.push_back(Incoming {
                        id: msg_id,
                        sender,
                        msg_type: MessageType::Broadcast { reliable },
                        msg: msg.msg.clone(),
                    })
                }
                self.next_id = msg_ids.next().unwrap();
            }
            MessageDestination::OneParty(destination) => {
                let next_id = self.next_id;
                self.next_id += 1;

                self.queue
                    .get_mut(usize::from(destination))
                    .ok_or(Reason::UnknownRecipient {
                        sender,
                        recipient: destination,
                    })?
                    .push_back(Incoming {
                        id: next_id,
                        sender,
                        msg_type: MessageType::P2P,
                        msg: msg.msg,
                    })
            }
        }

        Ok(())
    }

    fn recv_next_msg(&mut self, recipient: u16) -> Option<Incoming<M>> {
        self.queue[usize::from(recipient)].pop_front()
    }
}

/// Simulates execution of the protocol
///
/// Takes amount of participants, and a function that carries out the protocol for
/// one party. The function takes as input: index of the party, and [`MpcParty`](crate::MpcParty)
/// that can be used to communicate with others.
///
/// ## Example
/// ```rust,no_run
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
/// let output = round_based::sim::run(
///     n,
///     |i, party| protocol_of_random_generation(party, i, n),
/// )
/// .unwrap()
/// // unwrap `Result`s
/// .expect_ok()
/// // check that all parties produced the same response
/// .expect_eq();
///
/// println!("Output randomness: {}", hex::encode(output));
/// ```
pub fn run<M, F>(
    n: u16,
    mut party_start: impl FnMut(u16, crate::state_machine::MpcParty<M>) -> F,
) -> Result<SimResult<F::Output>, SimError>
where
    M: ProtocolMsg + Clone + 'static,
    F: Future,
{
    run_with_setup(core::iter::repeat_n((), n.into()), |i, party, ()| {
        party_start(i, party)
    })
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
/// let output = round_based::sim::run_with_setup(
///     core::iter::repeat_with(|| rng.fork()).take(n.into()),
///     |i, party, rng| protocol_of_random_generation(rng, party, i, n),
/// )
/// .unwrap()
/// // unwrap `Result`s
/// .expect_ok()
/// // check that all parties produced the same response
/// .expect_eq();
///
/// println!("Output randomness: {}", hex::encode(output));
/// ```
pub fn run_with_setup<S, M, F>(
    setups: impl IntoIterator<Item = S>,
    mut party_start: impl FnMut(u16, crate::state_machine::MpcParty<M>, S) -> F,
) -> Result<SimResult<F::Output>, SimError>
where
    M: ProtocolMsg + Clone + 'static,
    F: Future,
{
    let mut sim = Simulation::empty();

    for (setup, i) in setups.into_iter().zip(0u16..) {
        let party = crate::state_machine::wrap_protocol(|party| party_start(i, party, setup));
        sim.add_party(party);
    }

    sim.run()
}

#[cfg(test)]
mod tests {
    mod expect_eq {
        use crate::sim::SimResult;

        #[test]
        fn all_eq() {
            let res = SimResult::from(alloc::vec!["same string", "same string", "same string"])
                .expect_eq();
            assert_eq!(res, "same string")
        }

        #[test]
        #[should_panic]
        fn empty_res() {
            SimResult::from(alloc::vec![]).expect_eq()
        }

        #[test]
        #[should_panic]
        fn not_eq() {
            SimResult::from(alloc::vec![
                "one result",
                "one result",
                "another result",
                "one result",
                "and something else",
            ])
            .expect_eq();
        }
    }

    mod expect_ok {
        use crate::sim::SimResult;

        #[test]
        fn all_ok() {
            let res = SimResult::<Result<i32, core::convert::Infallible>>::from(alloc::vec![
                Ok(0),
                Ok(1),
                Ok(2)
            ])
            .expect_ok()
            .into_vec();

            assert_eq!(res, [0, 1, 2]);
        }

        #[test]
        #[should_panic]
        fn not_ok() {
            SimResult::from(alloc::vec![
                Ok(0),
                Err("i couldn't do what you asked :("),
                Ok(2),
                Ok(3),
                Err("sorry I was pooping, what did you want?")
            ])
            .expect_ok();
        }
    }
}
