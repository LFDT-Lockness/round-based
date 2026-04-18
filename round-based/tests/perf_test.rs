#[cfg(feature = "perf-profiler")]
mod tests {
    use core::cell::RefCell;
    use round_based::{
        Mpc, MpcExecution, Outgoing, ProtocolMsg, RoundMsg, mpc::profiler::wrapper::PerfProfiler,
        round::RoundInfo,
    };
    use std::time::Duration;

    #[derive(Debug, Clone)]
    struct ManualEvent;

    struct MockMpc {
        manual_events: RefCell<Vec<ManualEvent>>,
    }

    /// Random Beacon Messages
    #[derive(Clone, Debug)]
    enum RandomBeaconMsg {
        Commit([u8; 32]), // Round 1
        Decommit,         // Round 2
    }

    impl ProtocolMsg for RandomBeaconMsg {
        fn round(&self) -> u16 {
            match self {
                RandomBeaconMsg::Commit(_) => 1,
                RandomBeaconMsg::Decommit => 2,
            }
        }
    }

    // Round 1 Marker
    struct Round1;
    impl RoundMsg<[u8; 32]> for RandomBeaconMsg {
        const ROUND: u16 = 1;
        fn to_protocol_msg(m: [u8; 32]) -> Self {
            RandomBeaconMsg::Commit(m)
        }
        fn from_protocol_msg(msg: Self) -> Result<[u8; 32], Self> {
            match msg {
                RandomBeaconMsg::Commit(m) => Ok(m),
                _ => Err(msg),
            }
        }
    }

    // Round 2 Marker
    struct Round2;
    impl RoundMsg<u64> for RandomBeaconMsg {
        const ROUND: u16 = 2;
        fn to_protocol_msg(_m: u64) -> Self {
            RandomBeaconMsg::Decommit
        }
        fn from_protocol_msg(msg: Self) -> Result<u64, Self> {
            match msg {
                RandomBeaconMsg::Decommit => Ok(0),
                _ => Err(msg),
            }
        }
    }

    impl RoundInfo for Round1 {
        type Msg = [u8; 32];
        type Output = Vec<[u8; 32]>;
        type Error = core::convert::Infallible;
    }
    impl RoundInfo for Round2 {
        type Msg = u64;
        type Output = Vec<u64>;
        type Error = core::convert::Infallible;
    }

    impl Mpc for MockMpc {
        type Msg = RandomBeaconMsg;
        type Exec = MockMpc;
        type SendErr = core::convert::Infallible;
        fn add_round<R>(&mut self, _round: R) -> <Self::Exec as MpcExecution>::Round<R>
        where
            R: round_based::round::RoundStore,
            Self::Msg: RoundMsg<R::Msg>,
        {
        }
        fn finish_setup(self) -> Self::Exec {
            self
        }
    }

    impl MpcExecution for MockMpc {
        type Round<R: RoundInfo> = ();
        type Msg = RandomBeaconMsg;
        type CompleteRoundErr<E> = core::convert::Infallible;
        type SendErr = core::convert::Infallible;
        type SendMany = MockSendMany;

        async fn complete<R>(
            &mut self,
            _round: Self::Round<R>,
        ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
        where
            R: RoundInfo,
            Self::Msg: RoundMsg<R::Msg>,
        {
            tokio::time::sleep(Duration::from_millis(40)).await;
            self.manual_events.borrow_mut().push(ManualEvent);

            let res = Vec::<R::Msg>::new();
            let ptr = Box::into_raw(Box::new(res));
            Ok(unsafe { *Box::from_raw(ptr as *mut R::Output) })
        }

        async fn send(&mut self, _msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.manual_events.borrow_mut().push(ManualEvent);
            Ok(())
        }

        fn send_many(self) -> Self::SendMany {
            MockSendMany {
                manual_events: self.manual_events,
            }
        }

        async fn yield_now(&self) {
            tokio::time::sleep(Duration::from_millis(10)).await;
            self.manual_events.borrow_mut().push(ManualEvent);
        }
    }

    struct MockSendMany {
        manual_events: RefCell<Vec<ManualEvent>>,
    }
    impl round_based::mpc::SendMany for MockSendMany {
        type Exec = MockMpc;
        type Msg = RandomBeaconMsg;
        type SendErr = core::convert::Infallible;
        async fn send(&mut self, _msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.manual_events.borrow_mut().push(ManualEvent);
            Ok(())
        }
        async fn flush(self) -> Result<Self::Exec, Self::SendErr> {
            Ok(MockMpc {
                manual_events: self.manual_events,
            })
        }
    }

    /// Random Beacon Example
    async fn run_random_beacon(mut mpc: PerfProfiler<MockMpc>) -> [u8; 32] {
        // --- Round 1: Commit ---
        tokio::time::sleep(Duration::from_millis(10)).await;
        mpc.send(Outgoing::all_parties(RandomBeaconMsg::Commit([1u8; 32])))
            .await
            .ok();
        let _hashes = mpc.complete::<Round1>(()).await.expect("round 1");

        // --- Round 2: Reveal ---
        tokio::time::sleep(Duration::from_millis(5)).await;
        mpc.send(Outgoing::all_parties(RandomBeaconMsg::Decommit))
            .await
            .ok();
        let _numbers = mpc.complete::<Round2>(()).await.expect("round 2");

        // --- Final: XOR ---
        tokio::time::sleep(Duration::from_millis(5)).await;

        // --- Yield: Let others run ---
        mpc.yield_now().await;

        [0u8; 32]
    }

    #[tokio::test]
    async fn test_profiler_random_beacon() {
        let inner = MockMpc {
            manual_events: RefCell::new(Vec::new()),
        };
        let (profiler, handle) = PerfProfiler::new(inner);

        let _result = run_random_beacon(profiler).await;

        let report = handle.into_report();
        println!("{}", report);

        let r1 = report.rounds.iter().find(|r| r.round == 1).unwrap();
        let r2 = report.rounds.iter().find(|r| r.round == 2).unwrap();
        let r0 = report.rounds.iter().find(|r| r.round == 0).unwrap();

        assert!(r1.computation_time >= Duration::from_millis(10));
        assert!(r1.sent_io_time >= Duration::from_millis(20));
        assert!(r1.recv_io_time >= Duration::from_millis(40));

        assert!(r2.computation_time >= Duration::from_millis(5));
        assert!(r2.sent_io_time >= Duration::from_millis(20));
        assert!(r2.recv_io_time >= Duration::from_millis(40));

        // Yield Check
        assert!(r0.yield_time >= Duration::from_millis(10));
    }
}
