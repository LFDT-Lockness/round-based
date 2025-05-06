use std::convert::Infallible;

use futures::{sink, stream, SinkExt};
use hex_literal::hex;
use matches::assert_matches;
use rand_chacha::rand_core::SeedableRng;

use random_generation_protocol::{
    protocol_of_random_generation, CommitMsg, DecommitMsg, Error, Msg,
};
use round_based::{mpc::party::CompleteRoundError, Incoming, MessageType};

const PARTY0_SEED: [u8; 32] =
    hex!("6772d079d5c984b3936a291e36b0d3dc6c474e36ed4afdfc973ef79a431ca870");
const PARTY1_COMMITMENT: [u8; 32] =
    hex!("2a8c585d9a80cb78bc226f4ab35a75c8e5834ff77a83f41cf6c893ea0f3b2aed");
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

#[tokio::test]
async fn random_generation_completes() {
    let output = run_protocol([
        Ok::<_, Infallible>(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY2_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 2,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY1_RANDOMNESS,
            }),
        }),
        Ok(Incoming {
            id: 3,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY2_RANDOMNESS,
            }),
        }),
    ])
    .await
    .unwrap();

    assert_eq!(output, PROTOCOL_OUTPUT);
}

#[tokio::test]
async fn protocol_terminates_with_error_if_party_broadcasts_msg_unreliably_at_round1() {
    let output = run_protocol([Ok::<_, Infallible>(Incoming {
        id: 0,
        sender: 1,
        msg_type: MessageType::Broadcast { reliable: false },
        msg: Msg::CommitMsg(CommitMsg {
            commitment: PARTY1_COMMITMENT.into(),
        }),
    })])
    .await;

    assert_matches!(
        output,
        Err(Error::Round1Receive(CompleteRoundError::ProcessMsg(_)))
    )
}

#[tokio::test]
async fn protocol_terminates_with_error_if_party_tries_to_overwrite_message_at_round1() {
    let output = run_protocol([
        Ok::<_, Infallible>(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY_OVERWRITES.into(),
            }),
        }),
    ])
    .await;

    assert_matches!(
        output,
        Err(Error::Round1Receive(CompleteRoundError::ProcessMsg(_)))
    )
}

#[tokio::test]
async fn protocol_terminates_with_error_if_party_tries_to_overwrite_message_at_round2() {
    let output = run_protocol([
        Ok::<_, Infallible>(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY2_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 2,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY1_RANDOMNESS,
            }),
        }),
        Ok(Incoming {
            id: 3,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY_OVERWRITES,
            }),
        }),
    ])
    .await;

    assert_matches!(
        output,
        Err(Error::Round2Receive(CompleteRoundError::ProcessMsg(_)))
    )
}

#[tokio::test]
async fn protocol_terminates_if_received_message_from_unknown_sender_at_round1() {
    let output = run_protocol([Ok::<_, Infallible>(Incoming {
        id: 0,
        sender: 3,
        msg_type: MessageType::Broadcast { reliable: true },
        msg: Msg::CommitMsg(CommitMsg {
            commitment: PARTY1_COMMITMENT.into(),
        }),
    })])
    .await;

    assert_matches!(
        output,
        Err(Error::Round1Receive(CompleteRoundError::ProcessMsg(_)))
    )
}

#[tokio::test]
async fn protocol_ignores_message_that_goes_to_completed_round() {
    let output = run_protocol([
        Ok::<_, Infallible>(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY2_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 2,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY_OVERWRITES.into(),
            }),
        }),
        Ok(Incoming {
            id: 3,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY1_RANDOMNESS,
            }),
        }),
        Ok(Incoming {
            id: 4,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY2_RANDOMNESS,
            }),
        }),
    ])
    .await
    .unwrap();

    assert_eq!(output, PROTOCOL_OUTPUT);
}

#[tokio::test]
async fn protocol_ignores_io_error_if_it_is_completed() {
    let output = run_protocol([
        Ok(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY2_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 2,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY1_RANDOMNESS,
            }),
        }),
        Ok(Incoming {
            id: 3,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY2_RANDOMNESS,
            }),
        }),
        Err(DummyError),
    ])
    .await
    .unwrap();

    assert_eq!(output, PROTOCOL_OUTPUT);
}

#[tokio::test]
async fn protocol_terminates_with_error_if_io_error_happens_at_round2() {
    let output = run_protocol([
        Ok(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY2_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 2,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY1_RANDOMNESS,
            }),
        }),
        Err(DummyError),
        Ok(Incoming {
            id: 3,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY2_RANDOMNESS,
            }),
        }),
    ])
    .await;

    assert_matches!(output, Err(Error::Round2Receive(CompleteRoundError::Io(_))));
}

#[tokio::test]
async fn protocol_terminates_with_error_if_io_error_happens_at_round1() {
    let output = run_protocol([
        Err(DummyError),
        Ok(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY2_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 2,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY1_RANDOMNESS,
            }),
        }),
        Ok(Incoming {
            id: 3,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY2_RANDOMNESS,
            }),
        }),
    ])
    .await;

    assert_matches!(output, Err(Error::Round1Receive(CompleteRoundError::Io(_))));
}

#[tokio::test]
async fn protocol_terminates_with_error_if_unexpected_eof_happens_at_round2() {
    let output = run_protocol([
        Ok::<_, Infallible>(Incoming {
            id: 0,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY1_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 1,
            sender: 2,
            msg_type: MessageType::Broadcast { reliable: true },
            msg: Msg::CommitMsg(CommitMsg {
                commitment: PARTY2_COMMITMENT.into(),
            }),
        }),
        Ok(Incoming {
            id: 2,
            sender: 1,
            msg_type: MessageType::Broadcast { reliable: false },
            msg: Msg::DecommitMsg(DecommitMsg {
                randomness: PARTY1_RANDOMNESS,
            }),
        }),
    ])
    .await;

    assert_matches!(
        output,
        Err(Error::Round2Receive(CompleteRoundError::UnexpectedEof))
    );
}

async fn run_protocol<E, I>(
    incomings: I,
) -> Result<
    [u8; 32],
    random_generation_protocol::Error<
        round_based::mpc::party::CompleteRoundError<round_based::round::RoundInputError, E>,
        E,
    >,
>
where
    I: IntoIterator<Item = Result<Incoming<Msg>, E>>,
    I::IntoIter: Send + 'static,
    E: std::error::Error + Send + Sync + Unpin + 'static,
{
    let rng = rand_chacha::ChaCha8Rng::from_seed(PARTY0_SEED);

    let party = round_based::mpc::connected_halves(
        stream::iter(incomings),
        sink::drain().sink_map_err(|e| match e {}),
    );
    protocol_of_random_generation(party, 0, 3, rng).await
}

#[derive(Debug)]
struct DummyError;

impl std::fmt::Display for DummyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dummy error")
    }
}

impl std::error::Error for DummyError {}
