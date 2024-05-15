use alloc::{boxed::Box, collections::VecDeque, vec, vec::Vec};

use crate::{state_machine::ProceedResult, Incoming, MessageDestination, MessageType, Outgoing};

/// Simulates MPC protocol with parties defined as [state machines](crate::state_machine)
pub struct SimulationSync<O, M> {
    parties: Vec<Party<O, M>>,
}

enum Party<O, M> {
    Active {
        party: Box<dyn crate::state_machine::StateMachine<Output = O, Msg = M>>,
        wants_one_more_msg: bool,
    },
    Finished(O),
}

impl<O, M> SimulationSync<O, M>
where
    M: Clone + 'static,
{
    /// Creates empty simulation containing no parties
    ///
    /// New parties can be added via [`.add_party()`](Self::add_party)
    pub fn empty() -> Self {
        Self {
            parties: Vec::new(),
        }
    }

    /// Constructs a simulation with `n` parties
    ///
    /// Each party has index `0 <= i < n` and instantiated via provided `init` function
    pub fn from_fn<F>(
        n: u16,
        mut init: impl FnMut(u16, crate::state_machine::MpcParty<M>) -> F,
    ) -> Self
    where
        F: core::future::Future<Output = O> + 'static,
    {
        let mut sim = Self {
            parties: Vec::with_capacity(n.into()),
        };

        for i in 0..n {
            let party = crate::state_machine::wrap_protocol(|party| init(i, party));
            sim.add_party(party)
        }

        sim
    }

    /// Adds new party into the protocol
    ///
    /// New party will be assigned index `i = n - 1` where `n` is amount of parties in the
    /// simulation after this party was added.
    pub fn add_party(
        &mut self,
        party: impl crate::state_machine::StateMachine<Output = O, Msg = M> + 'static,
    ) {
        self.parties.push(Party::Active {
            party: Box::new(party),
            wants_one_more_msg: false,
        })
    }

    /// Returns amount of parties in the simulation
    pub fn parties_amount(&self) -> usize {
        self.parties.len()
    }

    /// Carries out the simulation
    pub fn run(mut self) -> Result<Vec<O>, SimulationSyncError> {
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
                            messages_queue.send_message(i, msg);
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

        Ok(self
            .parties
            .into_iter()
            .map(|party| match party {
                Party::Active { .. } => {
                    unreachable!("there must be no active parties when `parties_left == 0`")
                }
                Party::Finished(out) => out,
            })
            .collect())
    }
}

/// Error returned by [`SimulationSync::run`]
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct SimulationSyncError(Reason);

#[derive(Debug, thiserror::Error)]
enum Reason {
    #[error("save incoming message")]
    SaveIncomingMsg,
    #[error("execution error")]
    ExecutionError(#[source] crate::state_machine::ExecutionError),
}

impl From<Reason> for SimulationSyncError {
    fn from(err: Reason) -> Self {
        Self(err)
    }
}

struct MessagesQueue<M> {
    queue: Vec<VecDeque<Incoming<M>>>,
    next_id: u64,
}

impl<M: Clone> MessagesQueue<M> {
    fn new(n: usize) -> Self {
        Self {
            queue: vec![VecDeque::new(); n],
            next_id: 0,
        }
    }

    fn send_message(&mut self, sender: u16, msg: Outgoing<M>) {
        match msg.recipient {
            MessageDestination::AllParties => {
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
                        msg_type: MessageType::Broadcast,
                        msg: msg.msg.clone(),
                    })
                }
                self.next_id = msg_ids.next().unwrap();
            }
            MessageDestination::OneParty(destination) => {
                let next_id = self.next_id;
                self.next_id += 1;

                self.queue[usize::from(destination)].push_back(Incoming {
                    id: next_id,
                    sender,
                    msg_type: MessageType::P2P,
                    msg: msg.msg,
                })
            }
        }
    }

    fn recv_next_msg(&mut self, recipient: u16) -> Option<Incoming<M>> {
        self.queue[usize::from(recipient)].pop_front()
    }
}
