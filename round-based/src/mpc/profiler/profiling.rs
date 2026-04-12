use std::fmt;
use std::time::Duration;

/// Statistics for a single round of an MPC protocol.
#[derive(Debug, Clone, Default)]
pub struct RoundStats {
    /// Number of the round.
    pub round: usize,
    /// Time spent on computation during this round.
    pub computation_time: Duration,
    /// Time spent on I/O operations during this round.
    pub io_time: Duration,
}

/// A full performance report for a single protocol execution.
#[derive(Debug, Clone, Default)]
pub struct PerfReport {
    /// Statistics for each round.
    pub rounds: Vec<RoundStats>,
}

impl PerfReport {
    /// Calculates the total computation time across all rounds.
    pub fn total_computation(&self) -> Duration {
        self.rounds.iter().map(|r| r.computation_time).sum()
    }

    /// Calculates the total I/O time across all rounds.
    pub fn total_io(&self) -> Duration {
        self.rounds.iter().map(|r| r.io_time).sum()
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
                "Round {}: Computation: {:?}, I/O: {:?}",
                stat.round, stat.computation_time, stat.io_time
            )?;
        }
        writeln!(f, "------------------------------")?;
        writeln!(f, "Total Computation: {:?}", self.total_computation())?;
        writeln!(f, "Total I/O:         {:?}", self.total_io())?;
        writeln!(f, "Total Time:        {:?}", self.total_time())?;
        Ok(())
    }
}
