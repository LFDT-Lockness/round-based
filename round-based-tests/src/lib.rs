use round_based::{state_machine::ProceedResult, Incoming, Outgoing};

/// Wraps a state machine and provides convenient methods for feeding to and receiving messages from
/// the state machine, removing a boilerplate for handling `Yield`-ing, and providing convenient
/// methods for output assertions like `output.expect_eq()`
pub struct PartySim<S>(S);

pub fn new_one_party_sim<'a, M, F>(
    protocol: impl FnOnce(round_based::state_machine::MpcParty<M>) -> F,
) -> PartySim<impl round_based::state_machine::StateMachine<Msg = M, Output = F::Output> + 'a>
where
    M: round_based::ProtocolMsg + 'static,
    F: core::future::Future + 'a,
{
    PartySim(round_based::state_machine::wrap_protocol(protocol))
}

impl<S: round_based::state_machine::StateMachine> PartySim<S> {
    #[track_caller]
    pub fn receives(&mut self, msg: Incoming<S::Msg>) {
        loop {
            match self.0.proceed() {
                ProceedResult::NeedsOneMoreMessage => {
                    // that's exactly what we want
                    break;
                }
                ProceedResult::Yielded => {
                    // Protocol yielded, we ignore that
                    continue;
                }
                // everything else is unexpected
                r => panic!("state machine proceed: expected NeedsOneMoreMessage, got {r:?}"),
            }
        }
        self.0
            .received_msg(msg)
            .ok()
            .expect("couldn't feed a message into simulation")
    }

    #[track_caller]
    pub fn sends(&mut self) -> Expect<Outgoing<S::Msg>> {
        loop {
            match self.0.proceed() {
                ProceedResult::SendMsg(m) => break Expect(m),
                ProceedResult::Yielded => continue,
                r => panic!("state machine proceed: expected SendMsg, got {r:?}"),
            }
        }
    }

    #[track_caller]
    pub fn outputs(&mut self) -> Expect<S::Output> {
        loop {
            match self.0.proceed() {
                ProceedResult::Output(r) => break Expect(r),
                ProceedResult::Yielded => continue,
                r => panic!("state machine proceed: expected Output, got {r:?}"),
            }
        }
    }
}

#[must_use = "you need to make sure the output meets tests expectations"]
pub struct Expect<T>(pub T);

impl<T: Eq + core::fmt::Debug> Expect<T> {
    #[track_caller]
    pub fn expect_eq(&self, expected: &T) {
        assert_eq!(self.0, *expected)
    }
}

impl<T, E: core::fmt::Debug> Expect<Result<T, E>> {
    #[track_caller]
    pub fn unwrap(self) -> Expect<T> {
        Expect(self.0.unwrap())
    }
}
impl<T: core::fmt::Debug, E> Expect<Result<T, E>> {
    #[track_caller]
    pub fn unwrap_err(self) -> Expect<E> {
        Expect(self.0.unwrap_err())
    }
}
