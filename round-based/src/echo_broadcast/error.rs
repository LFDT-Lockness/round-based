#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub struct EchoError(#[from] Reason);

#[derive(thiserror::Error, Debug)]
pub(super) enum Reason {
    #[error(
        "round store received two msgs from same party and \
        didn't return an error"
    )]
    StoreReceivedTwoMsgsFromSameParty,
    #[error("received a msg from principal protocol when round is over")]
    ReceivedMainMsgWhenRoundOver,
    #[error("unknown sender i={i} (n={n})")]
    UnknownSender { i: u16, n: usize },
    #[error("local party index is out of bounds, probably indicates a bug")]
    OwnIndexOutOfBounds { i: u16, n: usize },
    #[error("principal round is finished, but store doesn't output")]
    MainRoundFinishedButStoreDoesntOutput,

    #[error("handle incoming echo msg")]
    HandleEcho(#[source] crate::round::RoundInputError),

    #[error("reliability check error: messages were not reliably broadcasted")]
    MismatchedHash,

    #[error("impossible state (it's a bug)")]
    StateGone,

    #[error("main round is in unexpected state (it's a bug)")]
    UnexpectedMainRoundState,

    #[error("clone error msg: RoundMsg implementation is incorrect")]
    RoundMsgClone,

    #[error(
        "protocol attempts to send a broadcast msg twice within the same round, it's unsupported"
    )]
    SendTwice,

    #[error("cannot convert a sent round msg back from proto msg (it's a bug)")]
    SentMsgFromProto,

    #[error(
        "sent a message that doesn't require reliable broadcast in reliable broadcast \
        round (round: {round}, dest: {dest:?})"
    )]
    SentNonReliableMsgInReliableRound {
        dest: crate::MessageDestination,
        round: u16,
    },

    #[error(
        "sent a reliable broadcast message in a regular round that doesn't require \
        reliable broadcast: you might have forgotten to register a reliable broadcast \
        round, or round store doesn't expose a property required to identify a reliable \
        broadcast round (round: {round})"
    )]
    SentReliableMsgInNonReliableRound { round: u16 },
}

#[derive(thiserror::Error, Debug)]
pub enum Error<E> {
    #[error("error originated in principal protocol")]
    Main(#[source] E),
    #[error("echo broadcast")]
    Echo(#[from] EchoError),
}

impl<E> From<Reason> for Error<E> {
    fn from(value: Reason) -> Self {
        Error::Echo(value.into())
    }
}

#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub struct CompleteRoundError<CompleteErr, SendErr>(
    #[from] CompleteRoundReason<CompleteErr, SendErr>,
);

#[derive(thiserror::Error, Debug)]
pub(super) enum CompleteRoundReason<CompleteErr, SendErr> {
    #[error(transparent)]
    CompleteRound(CompleteErr),
    #[error(transparent)]
    Send(SendErr),
    #[error(transparent)]
    Echo(Reason),
}

impl<A, B> From<Reason> for CompleteRoundError<A, B> {
    fn from(err: Reason) -> Self {
        CompleteRoundError(CompleteRoundReason::Echo(err))
    }
}

impl<A, B> From<EchoError> for CompleteRoundError<A, B> {
    fn from(err: EchoError) -> Self {
        err.0.into()
    }
}
