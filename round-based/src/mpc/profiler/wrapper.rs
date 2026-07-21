use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::vec::Vec;

use crate::{
    Mpc, MpcExecution, Outgoing, ProtocolMsg, RoundMsg,
    mpc::SendMany,
    round::{RoundInfo, RoundStore},
};

use super::profiling::{Event, PerfReport};

/// A handle to the performance profiler that can be used to generate a report
/// even after the profiler itself has been consumed.
#[derive(Clone)]
pub struct PerfProfilerHandle {
    events: Arc<Mutex<Vec<Event>>>,
    start_time: Instant,
}

impl PerfProfilerHandle {
    /// Consumes the handle and returns the performance report.
    pub fn into_report(self) -> PerfReport {
        let end_time = Instant::now();
        let events = self.events.lock().unwrap().clone();
        PerfReport::from_events(self.start_time, end_time, events)
    }
}

/// A wrapper around an MPC engine or execution that measures performance.
///
/// It stores a sequence of events (I/O and Yield) and uses them to calculate performance stats.
pub struct PerfProfiler<M> {
    inner: M,
    events: Arc<Mutex<Vec<Event>>>,
    start_time: Instant,
}

impl<M> PerfProfiler<M> {
    /// Creates a new performance profiler and a handle to retrieve the report.
    pub fn new(inner: M) -> (Self, PerfProfilerHandle) {
        let start_time = Instant::now();
        let events = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                inner,
                events: events.clone(),
                start_time,
            },
            PerfProfilerHandle { events, start_time },
        )
    }

    /// Consumes the profiler and returns the performance report.
    pub fn into_report(self) -> PerfReport {
        let end_time = Instant::now();
        let events = self.events.lock().unwrap().clone();
        PerfReport::from_events(self.start_time, end_time, events)
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
        self.inner.add_round(round)
    }

    fn finish_setup(self) -> Self::Exec {
        PerfProfiler {
            inner: self.inner.finish_setup(),
            events: self.events,
            start_time: self.start_time,
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
        let started = Instant::now();
        let result = self.inner.complete(round).await;
        let finished = Instant::now();

        let round_idx = <Self::Msg as RoundMsg<R::Msg>>::ROUND;
        self.events.lock().unwrap().push(Event::RecvMsgs {
            round: round_idx,
            started,
            finished,
        });

        result
    }

    async fn send(&mut self, msg: Outgoing<Self::Msg>) -> Result<(), Self::SendErr> {
        let round_idx = msg.msg.round();
        let started = Instant::now();
        let result = self.inner.send(msg).await;
        let finished = Instant::now();

        self.events.lock().unwrap().push(Event::SendMsg {
            round: round_idx,
            started,
            finished,
        });

        result
    }

    fn send_many(self) -> Self::SendMany {
        ProfilerSendMany {
            inner: self.inner.send_many(),
            events: self.events,
            start_time: self.start_time,
        }
    }

    async fn yield_now(&self) {
        let started = Instant::now();
        self.inner.yield_now().await;
        let finished = Instant::now();

        self.events
            .lock()
            .unwrap()
            .push(Event::Yielded { started, finished });
    }
}

/// A wrapper around [`SendMany`] that measures performance.
pub struct ProfilerSendMany<S> {
    inner: S,
    events: Arc<Mutex<Vec<Event>>>,
    start_time: Instant,
}

impl<S: SendMany> SendMany for ProfilerSendMany<S>
where
    S::Msg: ProtocolMsg,
{
    type Exec = PerfProfiler<S::Exec>;
    type Msg = S::Msg;
    type SendErr = S::SendErr;

    async fn send(&mut self, msg: Outgoing<S::Msg>) -> Result<(), S::SendErr> {
        let round_idx = msg.msg.round();
        let started = Instant::now();
        let result = self.inner.send(msg).await;
        let finished = Instant::now();

        self.events.lock().unwrap().push(Event::SendMsg {
            round: round_idx,
            started,
            finished,
        });

        result
    }

    async fn flush(self) -> Result<Self::Exec, S::SendErr> {
        let result = self.inner.flush().await;

        Ok(PerfProfiler {
            inner: result?,
            events: self.events,
            start_time: self.start_time,
        })
    }
}
