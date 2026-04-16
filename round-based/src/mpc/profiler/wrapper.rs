use core::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use crate::{
    Mpc, MpcExecution, Outgoing, ProtocolMsg, RoundMsg,
    mpc::SendMany,
    round::{RoundInfo, RoundStore},
};

use super::profiling::PerfReport;

/// A wrapper around an MPC engine or execution that measures performance.
///
/// It measures computation time (time between MPC calls) and I/O time (time spent inside MPC calls).
pub struct PerfProfiler<M> {
    inner: M,
    report: RefCell<PerfReport>,
    last_resume: Cell<Instant>,
}

impl<M> PerfProfiler<M> {
    /// Creates a new performance profiler.
    pub fn new(inner: M) -> Self {
        Self {
            inner,
            report: RefCell::new(PerfReport::default()),
            last_resume: Cell::new(Instant::now()),
        }
    }

    /// Consumes the profiler and returns the performance report.
    pub fn into_report(self) -> PerfReport {
        let elapsed = self.last_resume.get().elapsed();
        if elapsed != Duration::ZERO {
            // Attribute trailing time to round 0 (Global/Teardown)
            self.report.borrow_mut().apply_stats(
                0,
                elapsed,
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
            );
        }
        self.report.into_inner()
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

    fn update_report(
        &self,
        round: usize,
        comp_time: Duration,
        sent_io: Duration,
        recv_io: Duration,
        yield_time: Duration,
    ) {
        self.report
            .borrow_mut()
            .apply_stats(round, comp_time, sent_io, recv_io, yield_time);
    }
}

impl<M: Mpc> Mpc for PerfProfiler<M>
where
    M::Msg: ProtocolMsg,
{
    type Msg = M::Msg;
    type Exec = PerfProfiler<M::Exec>;
    type SendErr = M::SendErr;

    fn add_round<R>(&mut self, round: R) -> <Self::Exec as MpcExecution>::Round<R>
    where
        R: RoundStore,
        Self::Msg: RoundMsg<R::Msg>,
    {
        let elapsed = self.last_resume.get().elapsed();
        let round_idx = <Self::Msg as RoundMsg<R::Msg>>::ROUND as usize;
        self.update_report(
            round_idx,
            elapsed,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        );

        let res = self.inner.add_round(round);
        self.last_resume.set(Instant::now());
        res
    }

    fn finish_setup(self) -> Self::Exec {
        let elapsed = self.last_resume.get().elapsed();
        if elapsed != Duration::ZERO {
            // Attribute setup completion time to round 0
            self.update_report(0, elapsed, Duration::ZERO, Duration::ZERO, Duration::ZERO);
        }

        PerfProfiler {
            inner: self.inner.finish_setup(),
            report: self.report,
            last_resume: Cell::new(Instant::now()),
        }
    }
}

impl<M: MpcExecution> MpcExecution for PerfProfiler<M>
where
    M::Msg: ProtocolMsg,
{
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
        let comp_time = self.last_resume.get().elapsed();

        let io_start = Instant::now();
        let result = self.inner.complete(round).await;
        let io_time = io_start.elapsed();

        let round_idx = <Self::Msg as RoundMsg<R::Msg>>::ROUND as usize;
        self.update_report(
            round_idx,
            comp_time,
            Duration::ZERO,
            io_time,
            Duration::ZERO,
        );

        self.last_resume.set(Instant::now());
        result
    }

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        let comp_time = self.last_resume.get().elapsed();
        let round_idx = msg.msg.round() as usize;

        let io_start = Instant::now();
        let result = self.inner.send(msg).await;
        let io_time = io_start.elapsed();

        self.update_report(
            round_idx,
            comp_time,
            io_time,
            Duration::ZERO,
            Duration::ZERO,
        );

        self.last_resume.set(Instant::now());
        result
    }

    fn send_many(self) -> Self::SendMany {
        ProfilerSendMany {
            inner: self.inner.send_many(),
            report: self.report,
            last_resume: self.last_resume,
        }
    }

    async fn yield_now(&self) {
        let comp_time = self.last_resume.get().elapsed();

        let start = Instant::now();
        self.inner.yield_now().await;
        let yield_time = start.elapsed();

        // Attribute yield to round 0
        self.update_report(0, comp_time, Duration::ZERO, Duration::ZERO, yield_time);
        self.last_resume.set(Instant::now());
    }
}

/// A wrapper around [`SendMany`] that measures performance.
pub struct ProfilerSendMany<S> {
    inner: S,
    report: RefCell<PerfReport>,
    last_resume: Cell<Instant>,
}

impl<S: SendMany> SendMany for ProfilerSendMany<S>
where
    S::Msg: ProtocolMsg,
{
    type Exec = PerfProfiler<S::Exec>;
    type Msg = S::Msg;
    type SendErr = S::SendErr;

    async fn send(&mut self, msg: Outgoing<S::Msg>) -> Result<(), S::SendErr> {
        let comp_time = self.last_resume.get().elapsed();
        let round_idx = msg.msg.round() as usize;

        let io_start = Instant::now();
        let result = self.inner.send(msg).await;
        let io_time = io_start.elapsed();

        self.report.borrow_mut().apply_stats(
            round_idx,
            comp_time,
            io_time,
            Duration::ZERO,
            Duration::ZERO,
        );

        self.last_resume.set(Instant::now());
        result
    }

    async fn flush(self) -> Result<Self::Exec, S::SendErr> {
        let comp_time = self.last_resume.get().elapsed();

        let io_start = Instant::now();
        let result = self.inner.flush().await;
        let io_time = io_start.elapsed();

        // Attribute flush to round 0
        self.report
            .borrow_mut()
            .apply_stats(0, comp_time, io_time, Duration::ZERO, Duration::ZERO);

        Ok(PerfProfiler {
            inner: result?,
            report: self.report,
            last_resume: Cell::new(Instant::now()),
        })
    }
}
