#[cfg(feature = "std")]
mod tests {
    use round_based::{mpc::profiler::{wrapper::PerfProfiler, stats}, MpcExecution, Outgoing, RoundMsg, ProtocolMsg};
    use std::time::Duration;
    use std::thread;

    struct MockMpc;

    #[derive(Clone, Debug)]
    enum MockMsg {
        Round1(()),
    }

    impl ProtocolMsg for MockMsg {
        fn round(&self) -> u16 {
            match self {
                MockMsg::Round1(_) => 1,
            }
        }
    }

    impl RoundMsg<()> for MockMsg {
        const ROUND: u16 = 1;
        fn to_protocol_msg(m: ()) -> Self { MockMsg::Round1(m) }
        fn from_protocol_msg(protocol_msg: Self) -> Result<(), Self> {
            match protocol_msg {
                MockMsg::Round1(m) => Ok(m),
            }
        }
    }

    impl MpcExecution for MockMpc {
        type Round<R: round_based::round::RoundInfo> = ();
        type Msg = MockMsg;
        type CompleteRoundErr<E> = core::convert::Infallible;
        type SendErr = core::convert::Infallible;
        type SendMany = MockSendMany;

        async fn complete<R>(
            &mut self,
            _round: Self::Round<R>,
        ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
        where
            R: round_based::round::RoundInfo,
            Self::Msg: RoundMsg<R::Msg>,
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
            unreachable!()
        }

        async fn send(&mut self, _msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        }

        fn send_many(self) -> Self::SendMany {
            MockSendMany
        }

        async fn yield_now(&self) {}
    }

    struct MockSendMany;
    impl round_based::mpc::SendMany for MockSendMany {
        type Exec = MockMpc;
        type Msg = MockMsg;
        type SendErr = core::convert::Infallible;

        async fn send(&mut self, _msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        }

        async fn flush(self) -> Result<Self::Exec, Self::SendErr> {
            Ok(MockMpc)
        }
    }

    impl MockMpc {
        async fn simulate_round(&self) {
            // Simulate "Pure Computation"
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[tokio::test]
    async fn test_profiler_captures_correct_times() {
        let inner = MockMpc;
        let mut profiler = PerfProfiler::new(inner);

        // --- ROUND 1 ---
        // 1. Computation happens
        profiler.get_ref().simulate_round().await;
        
        // 2. I/O happens via send
        profiler.send(Outgoing::all_parties(MockMsg::Round1(()))).await.unwrap();

        let report = profiler.into_report();

        // Check if computation is at least 50ms
        assert!(report.total_computation() >= Duration::from_millis(50), "Computation time was {:?}", report.total_computation());
        // Check if I/O is at least 100ms
        assert!(report.total_io() >= Duration::from_millis(100), "IO time was {:?}", report.total_io());
        
        println!("{}", report);
    }

    #[test]
    fn test_statistical_analysis() {
        use round_based::mpc::profiler::{profiling::PerfReport, profiling::RoundStats};

        // Create dummy reports to test the math
        let mut reports = Vec::new();
        for i in 1..=10 {
            reports.push(PerfReport {
                rounds: vec![RoundStats {
                    round: 1,
                    computation_time: Duration::from_millis(i * 10), // 10, 20, ... 100
                    io_time: Duration::from_millis(50),
                }],
            });
        }

        // Capture total times
        let total_times: Vec<Duration> = reports.iter().map(|r| r.total_time()).collect();
        let analysis = stats::analyze_durations("Batch Execution", total_times);

        // Verification
        assert_eq!(analysis.metric_name, "Batch Execution");
        assert!(analysis.p50 >= Duration::from_millis(50));
        assert!(analysis.p90 >= Duration::from_millis(90));

        // Test the Display for stats
        let stats_output = format!("{}", analysis);
        assert!(stats_output.contains("Mean"));
        println!("{}", stats_output);
    }

    #[test]
    fn test_report_display_formatting() {
        use round_based::mpc::profiler::{profiling::PerfReport, profiling::RoundStats};

        let report = PerfReport {
            rounds: vec![
                RoundStats {
                    round: 1,
                    computation_time: Duration::from_millis(15),
                    io_time: Duration::from_millis(45),
                },
                RoundStats {
                    round: 2,
                    computation_time: Duration::from_millis(20),
                    io_time: Duration::from_millis(30),
                },
            ],
        };

        let output = format!("{}", report);
        assert!(output.contains("Round 1"));
        assert!(output.contains("Round 2"));
        assert!(output.contains("Total Time"));
    }
}
