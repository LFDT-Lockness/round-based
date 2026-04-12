use std::time::{Duration, Instant};

use crate::{MpcExecution, Outgoing, RoundMsg, mpc::SendMany, round::RoundInfo};

use super::profiling::{PerfReport, RoundStats};

/// Extension trait that allows to wrap any MPC execution with a performance profiler.
pub trait ProfilerExt: MpcExecution + Sized {
    /// Wraps the MPC execution with a performance profiler.
    fn profile(self) -> PerfProfiler<Self> {
        PerfProfiler::new(self)
    }
}

impl<M: MpcExecution> ProfilerExt for M {}

/// A wrapper around an MPC execution that measures performance.
///
/// It measures computation time (time between MPC calls) and I/O time (time spent inside MPC calls).
pub struct PerfProfiler<M> {
    inner: M,
    report: PerfReport,
    last_resume: Instant,
    current_round: usize,
    current_comp_time: Duration,
}

impl<M> PerfProfiler<M> {
    /// Creates a new performance profiler.
    pub fn new(inner: M) -> Self {
        Self {
            inner,
            report: PerfReport::default(),
            last_resume: Instant::now(),
            current_round: 1,
            current_comp_time: Duration::ZERO,
        }
    }

    /// Consumes the profiler and returns the performance report.
    pub fn into_report(mut self) -> PerfReport {
        self.current_comp_time += self.last_resume.elapsed();
        if self.current_comp_time != Duration::ZERO {
            self.update_report(self.current_round, self.current_comp_time, Duration::ZERO);
        }
        self.report
    }

    /// Returns a reference to the inner MPC execution.
    pub fn get_ref(&self) -> &M {
        &self.inner
    }

    /// Returns a mutable reference to the inner MPC execution.
    pub fn get_mut(&mut self) -> &mut M {
        &mut self.inner
    }

    /// Consumes the profiler and returns the inner MPC execution.
    pub fn into_inner(self) -> M {
        self.inner
    }

    fn update_report(&mut self, round: usize, comp_time: Duration, io_time: Duration) {
        if let Some(last) = self.report.rounds.last_mut()
            && last.round == round
        {
            last.computation_time += comp_time;
            last.io_time += io_time;
            return;
        }
        self.report.rounds.push(RoundStats {
            round,
            computation_time: comp_time,
            io_time,
        });
    }
}

impl<M: MpcExecution> MpcExecution for PerfProfiler<M> {
    type Round<R: RoundInfo> = M::Round<R>;
    type Msg = M::Msg;
    type CompleteRoundErr<E> = M::CompleteRoundErr<E>;
    type SendErr = M::SendErr;
    type SendMany = ProfilerSendMany<M::SendMany>;

    async fn complete<R>(
        &mut self,
        round: Self::Round<R>,
    ) -> Result<R::Output, Self::CompleteRoundErr<R::Error>>
    where
        R: RoundInfo,
        Self::Msg: RoundMsg<R::Msg>,
    {
        self.current_comp_time += self.last_resume.elapsed();

        let io_start = Instant::now();
        let result = self.inner.complete(round).await;
        let io_time = io_start.elapsed();

        let round_idx = <Self::Msg as RoundMsg<R::Msg>>::ROUND as usize;

        self.update_report(round_idx, self.current_comp_time, io_time);

        self.current_round = round_idx + 1;
        self.current_comp_time = Duration::ZERO;
        self.last_resume = Instant::now();

        result
    }

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        self.current_comp_time += self.last_resume.elapsed();

        let io_start = Instant::now();
        let result = self.inner.send(msg).await;
        let io_time = io_start.elapsed();

        self.update_report(self.current_round, self.current_comp_time, io_time);

        self.current_comp_time = Duration::ZERO;
        self.last_resume = Instant::now();

        result
    }

    fn send_many(self) -> Self::SendMany {
        ProfilerSendMany {
            inner: self.inner.send_many(),
            report: self.report,
            last_resume: self.last_resume,
            current_round: self.current_round,
            current_comp_time: self.current_comp_time,
        }
    }

    async fn yield_now(&self) {
        self.inner.yield_now().await;
    }
}

/// A wrapper around [`SendMany`] that measures performance.
pub struct ProfilerSendMany<S> {
    inner: S,
    report: PerfReport,
    last_resume: Instant,
    current_round: usize,
    current_comp_time: Duration,
}

impl<S: SendMany> SendMany for ProfilerSendMany<S> {
    type Exec = PerfProfiler<S::Exec>;
    type Msg = S::Msg;
    type SendErr = S::SendErr;

    async fn send(&mut self, msg: Outgoing<S::Msg>) -> Result<(), S::SendErr> {
        self.current_comp_time += self.last_resume.elapsed();

        let io_start = Instant::now();
        let result = self.inner.send(msg).await;
        let io_time = io_start.elapsed();

        self.update_report(self.current_round, self.current_comp_time, io_time);

        self.current_comp_time = Duration::ZERO;
        self.last_resume = Instant::now();

        result
    }

    async fn flush(self) -> Result<Self::Exec, S::SendErr> {
        let current_comp_time = self.current_comp_time + self.last_resume.elapsed();

        let io_start = Instant::now();
        let result = self.inner.flush().await;
        let io_time = io_start.elapsed();

        let mut report = self.report;
        let profiler_inner = result?;

        // We need to update the report before returning
        update_report_static(&mut report, self.current_round, current_comp_time, io_time);

        Ok(PerfProfiler {
            inner: profiler_inner,
            report,
            last_resume: Instant::now(),
            current_round: self.current_round,
            current_comp_time: Duration::ZERO,
        })
    }
}

impl<S> ProfilerSendMany<S> {
    fn update_report(&mut self, round: usize, comp_time: Duration, io_time: Duration) {
        update_report_static(&mut self.report, round, comp_time, io_time);
    }
}

fn update_report_static(
    report: &mut PerfReport,
    round: usize,
    comp_time: Duration,
    io_time: Duration,
) {
    if let Some(last) = report.rounds.last_mut()
        && last.round == round
    {
        last.computation_time += comp_time;
        last.io_time += io_time;
        return;
    }
    report.rounds.push(RoundStats {
        round,
        computation_time: comp_time,
        io_time,
    });
}
