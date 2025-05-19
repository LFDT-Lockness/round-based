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
    ReceivedPrincipalMsgWhenRoundOver,
    #[error("received an echo msg when round is over")]
    ReceivedEchoMsgWhenRoundOver,
    #[error("unknown sender i={i} (n={n})")]
    UnknownSender { i: u16, n: usize },
    #[error("local party index is out of bounds, probably indicates a bug")]
    OwnIndexOutOfBounds { i: u16, n: usize },
    #[error("principal round is finished, but store doesn't output")]
    PrincipalRoundFinishedButStoreDoesntOutput,
    #[error("echo round is finished, but store doesn't output")]
    EchoRoundFinishedButStoreDoesntOutput,

    #[error("handle incoming echo msg")]
    HandleEcho(#[source] crate::round::RoundInputError),

    #[error("reliability check error: messages were not reliably broadcasted")]
    MismatchedHash,

    #[error("round has already returned output or error")]
    StateFinished,
    #[error("impossible state (it's a bug)")]
    StateGone,

    #[error("main round is in unexpected state (it's a bug)")]
    UnexpectedMainRoundState,
}

#[derive(thiserror::Error, Debug)]
pub enum Error<E> {
    #[error("error originated in principal protocol")]
    Principal(#[source] E),
    #[error("echo broadcast")]
    Echo(#[from] EchoError),
}

impl<E> From<Reason> for Error<E> {
    fn from(value: Reason) -> Self {
        Error::Echo(value.into())
    }
}
