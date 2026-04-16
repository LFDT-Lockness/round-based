use std::fmt;
use std::time::Duration;
use std::vec::Vec;

/// Statistics for a single round of an MPC protocol.
#[derive(Debug, Clone, Default)]
pub struct RoundStats {
    /// Number of the round.
    pub round: usize,
    /// Time spent on computation during this round.
    pub computation_time: Duration,
    /// Time spent on sending messages during this round.
    pub sent_io_time: Duration,
    /// Time spent on receiving messages during this round.
    pub recv_io_time: Duration,
    /// Time spent on waiting for the scheduler (yield_now).
    pub yield_time: Duration,
}

/// A full performance report for a single protocol execution.
#[derive(Debug, Clone, Default)]
pub struct PerfReport {
    /// Statistics for each round.
    pub rounds: Vec<RoundStats>,
}

impl PerfReport {
    /// Applies new statistics to the report.
    ///
    /// If an entry for the same round already exists, the statistics are added to it.
    /// Otherwise, a new entry is created.
    pub fn apply_stats(
        &mut self,
        round: usize,
        computation: Duration,
        sent_io: Duration,
        recv_io: Duration,
        yield_time: Duration,
    ) {
        if let Some(existing) = self.rounds.iter_mut().find(|r| r.round == round) {
            existing.computation_time += computation;
            existing.sent_io_time += sent_io;
            existing.recv_io_time += recv_io;
            existing.yield_time += yield_time;
            return;
        }
        self.rounds.push(RoundStats {
            round,
            computation_time: computation,
            sent_io_time: sent_io,
            recv_io_time: recv_io,
            yield_time,
        });
    }

    /// Calculates the total computation time across all rounds.
    pub fn total_computation(&self) -> Duration {
        self.rounds.iter().map(|r| r.computation_time).sum()
    }

    /// Calculates the total I/O time spent on sending across all rounds.
    pub fn total_sent_io(&self) -> Duration {
        self.rounds.iter().map(|r| r.sent_io_time).sum()
    }

    /// Calculates the total I/O time spent on receiving across all rounds.
    pub fn total_recv_io(&self) -> Duration {
        self.rounds.iter().map(|r| r.recv_io_time).sum()
    }

    /// Calculates the total I/O time spent on yielding across all rounds.
    pub fn total_yield(&self) -> Duration {
        self.rounds.iter().map(|r| r.yield_time).sum()
    }

    /// Calculates the total I/O time across all rounds (send + recv + yield).
    pub fn total_io(&self) -> Duration {
        self.total_sent_io() + self.total_recv_io() + self.total_yield()
    }

    /// Calculates the total execution time (computation + I/O).
    pub fn total_time(&self) -> Duration {
        self.total_computation() + self.total_io()
    }
}

impl fmt::Display for PerfReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== MPC Performance Report ===")?;
        for stat in &self.rounds {
            writeln!(
                f,
                "Round {}: Computation: {:?}, Sent I/O: {:?}, Recv I/O: {:?}, Yield: {:?}",
                stat.round,
                stat.computation_time,
                stat.sent_io_time,
                stat.recv_io_time,
                stat.yield_time
            )?;
        }
        writeln!(f, "------------------------------")?;
        writeln!(f, "Total Computation: {:?}", self.total_computation())?;
        writeln!(f, "Total Sent I/O:    {:?}", self.total_sent_io())?;
        writeln!(f, "Total Recv I/O:    {:?}", self.total_recv_io())?;
        writeln!(f, "Total Yield:       {:?}", self.total_yield())?;
        writeln!(f, "Total Time:        {:?}", self.total_time())?;
        Ok(())
    }
}
