use hex_literal::hex;
use matches::assert_matches;
use rand_chacha::rand_core::SeedableRng;

use random_generation_protocol::{protocol_of_random_generation, CommitMsg, DecommitMsg, Msg};
use round_based::{echo_broadcast as echo, Incoming, MessageType, Outgoing};

const PARTY0_SEED: [u8; 32] =
    hex!("6772d079d5c984b3936a291e36b0d3dc6c474e36ed4afdfc973ef79a431ca870");
const PARTY0_COMMITMENT: [u8; 32] =
    hex!("6ac69d1b2082536de5d4f5092f807873173fac86117cb7b5e179b191e97fc8d7");
const PARTY0_RANDOMNESS: [u8; 32] =
    hex!("15f88064c7daeb863f37c3a41466a827f5ca07b7f7dd8e58c510ff612e641ea2");
const PARTY1_COMMITMENT: [u8; 32] =
    hex!("2a8c585d9a80cb78bc226f4ab35a75c8e5834ff77a83f41cf6c893ea0f3b2aed");
const ECHO_MSG: [u8; 32] = hex!("b9c51267eae8dc5ea988111e933709fa8496b5cab4aa11778f61271527d3760a");
const PARTY1_RANDOMNESS: [u8; 32] =
    hex!("12a595f4893fdb4ab9cc38caeec5f7456acb3002ca58457c5056977ce59136a6");
const PARTY2_COMMITMENT: [u8; 32] =
    hex!("01274ef40aece8aa039587cc05620a19b80a5c93fbfb24a9f8e1b77b7936e47d");
const PARTY2_RANDOMNESS: [u8; 32] =
    hex!("6fc78a926c7eebfad4e98e796cd53b771ac5947b460567c7ea441abb957c89c7");
const PROTOCOL_OUTPUT: [u8; 32] =
    hex!("689a9f02229bdb36521275179676641585c4a3ce7b80ace37f0272a65e89a1c3");
const PARTY_OVERWRITES: [u8; 32] =
    hex!("00aa11bb22cc33dd44ee55ff6677889900aa11bb22cc33dd44ee55ff66778899");

#[test]
fn random_generation_completes() {
    let mut sim = simulation();
    // Round 0 - commitment
    sim.sends().expect_eq(&Outgoing {
        recipient: round_based::MessageDestination::AllParties { reliable: false },
        msg: echo::Msg::Main(Msg::CommitMsg(CommitMsg {
            commitment: PARTY0_COMMITMENT.into(),
        })),
    });
    sim.receives(Incoming {
        id: 0,
        sender: 1,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Main(Msg::CommitMsg(CommitMsg {
            commitment: PARTY1_COMMITMENT.into(),
        })),
    });
    sim.receives(Incoming {
        id: 1,
        sender: 2,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Main(Msg::CommitMsg(CommitMsg {
            commitment: PARTY2_COMMITMENT.into(),
        })),
    });
    // Round 1 - echo round
    sim.sends().expect_eq(&Outgoing {
        recipient: round_based::MessageDestination::AllParties { reliable: false },
        msg: echo::Msg::Echo {
            round: 0,
            hash: ECHO_MSG.into(),
        },
    });
    sim.receives(Incoming {
        id: 2,
        sender: 1,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Echo {
            round: 0,
            hash: ECHO_MSG.into(),
        },
    });
    sim.receives(Incoming {
        id: 3,
        sender: 2,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Echo {
            round: 0,
            hash: ECHO_MSG.into(),
        },
    });
    // Round 2 - decommitment
    sim.sends().expect_eq(&Outgoing {
        recipient: round_based::MessageDestination::AllParties { reliable: false },
        msg: echo::Msg::Main(Msg::DecommitMsg(DecommitMsg {
            randomness: PARTY0_RANDOMNESS.into(),
        })),
    });
    sim.receives(Incoming {
        id: 4,
        sender: 1,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Main(Msg::DecommitMsg(DecommitMsg {
            randomness: PARTY1_RANDOMNESS,
        })),
    });
    sim.receives(Incoming {
        id: 5,
        sender: 2,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Main(Msg::DecommitMsg(DecommitMsg {
            randomness: PARTY2_RANDOMNESS,
        })),
    });

    sim.outputs().unwrap().expect_eq(&PROTOCOL_OUTPUT);
}

#[test]
fn detects_unreliable_broadcast() {
    let mut sim = simulation();
    // Round 0 - commitment
    sim.sends().expect_eq(&Outgoing {
        recipient: round_based::MessageDestination::AllParties { reliable: false },
        msg: echo::Msg::Main(Msg::CommitMsg(CommitMsg {
            commitment: PARTY0_COMMITMENT.into(),
        })),
    });
    sim.receives(Incoming {
        id: 0,
        sender: 1,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Main(Msg::CommitMsg(CommitMsg {
            commitment: PARTY1_COMMITMENT.into(),
        })),
    });
    sim.receives(Incoming {
        id: 1,
        sender: 2,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Main(Msg::CommitMsg(CommitMsg {
            commitment: PARTY2_COMMITMENT.into(),
        })),
    });
    // Round 1 - echo round
    sim.sends().expect_eq(&Outgoing {
        recipient: round_based::MessageDestination::AllParties { reliable: false },
        msg: echo::Msg::Echo {
            round: 0,
            hash: ECHO_MSG.into(),
        },
    });
    sim.receives(Incoming {
        id: 2,
        sender: 1,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Echo {
            round: 0,
            hash: ECHO_MSG.into(),
        },
    });
    sim.receives(Incoming {
        id: 3,
        sender: 2,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: echo::Msg::Echo {
            round: 0,
            hash: PARTY_OVERWRITES.into(),
        },
    });

    assert_matches!(
        sim.outputs().unwrap_err().0,
        random_generation_protocol::Error::Round1Receive(echo::CompleteRoundError::Echo(err))
            if err.reliability_check_failed()
    );
}

fn simulation() -> round_based_tests::PartySim<
    impl round_based::state_machine::StateMachine<
        Msg = echo::Msg<sha2::Sha256, Msg>,
        Output = Result<
            [u8; 32],
            random_generation_protocol::Error<
                round_based::echo_broadcast::CompleteRoundError<
                    round_based::mpc::party::CompleteRoundError<
                        round_based::echo_broadcast::Error<round_based::round::RoundInputError>,
                        round_based::state_machine::DeliveryErr,
                    >,
                    round_based::state_machine::DeliveryErr,
                >,
                round_based::echo_broadcast::Error<round_based::state_machine::DeliveryErr>,
            >,
        >,
    >,
> {
    let rng = rand_chacha::ChaCha8Rng::from_seed(PARTY0_SEED);
    round_based_tests::new_one_party_sim(|party| async {
        let party = round_based::echo_broadcast::wrap(party, 0, 3);
        protocol_of_random_generation(party, 0, 3, rng).await
    })
}
