//! Statistics for MPC protocol execution.

use super::profiling::PerfReport;
use std::string::{String, ToString};
use std::time::Duration;
use std::vec::Vec;
use std::{format, println};

/// Aggregated statistics for a set of durations.
#[derive(Debug)]
pub struct AggregatedStats {
    /// Name of the metric.
    pub metric_name: String,
    /// Mean duration.
    pub mean: Duration,
    /// Standard deviation of durations.
    pub std_dev: Duration,
    /// Median (50th percentile) duration.
    pub p50: Duration,
    /// 75th percentile duration.
    pub p75: Duration,
    /// 90th percentile duration.
    pub p90: Duration,
}

impl std::fmt::Display for AggregatedStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Table-like formatting: {:<N} left-aligns the value and pads it to N characters for a clean grid
        write!(
            f,
            "{:<20} | {:<12} | {:<12} | {:<12} | {:<12} | {:<12}",
            self.metric_name,
            format!("{:?}", self.mean),
            format!("{:?}", self.std_dev),
            format!("{:?}", self.p50),
            format!("{:?}", self.p75),
            format!("{:?}", self.p90)
        )
    }
}

/// Analyzes a set of durations and returns aggregated statistics.
pub fn analyze_durations(name: &str, mut durations: Vec<Duration>) -> AggregatedStats {
    if durations.is_empty() {
        return AggregatedStats {
            metric_name: name.to_string(),
            mean: Duration::ZERO,
            std_dev: Duration::ZERO,
            p50: Duration::ZERO,
            p75: Duration::ZERO,
            p90: Duration::ZERO,
        };
    }

    durations.sort_unstable();
    let len = durations.len();

    let sum: Duration = durations.iter().sum();
    let mean = sum / len as u32;

    let variance_secs = durations
        .iter()
        .map(|&d| {
            let diff = d.abs_diff(mean);
            diff.as_secs_f64().powi(2)
        })
        .sum::<f64>()
        / len as f64;

    let std_dev = Duration::from_secs_f64(variance_secs.sqrt());

    let p50 = durations[(len as f64 * 0.50).floor() as usize];
    let p75 = durations[(len as f64 * 0.75).floor() as usize];
    let p90 = durations[(len as f64 * 0.90).floor() as usize];

    AggregatedStats {
        metric_name: name.to_string(),
        mean,
        std_dev,
        p50,
        p75,
        p90,
    }
}

/// Helper to consume multiple reports and print aggregated analytics for all metrics.
pub fn analyze_reports(reports: &[PerfReport]) {
    if reports.is_empty() {
        println!("No reports to analyze.");
        return;
    }

    let mut total_times = Vec::with_capacity(reports.len());
    let mut comp_times = Vec::with_capacity(reports.len());
    let mut sent_io_times = Vec::with_capacity(reports.len());
    let mut recv_io_times = Vec::with_capacity(reports.len());
    let mut yield_times = Vec::with_capacity(reports.len());

    for report in reports {
        total_times.push(report.total_time());
        comp_times.push(report.total_computation());
        sent_io_times.push(report.total_sent_io());
        recv_io_times.push(report.total_recv_io());
        yield_times.push(report.total_yield());
    }

    println!("\n=== MPC Execution Analytics ({} runs) ===", reports.len());
    println!(
        "{:<20} | {:<12} | {:<12} | {:<12} | {:<12} | {:<12}",
        "Metric", "Mean", "Std Dev", "p50", "p75", "p90"
    );
    println!("{}", "-".repeat(90));
    println!("{}", analyze_durations("Total Time", total_times));
    println!("{}", analyze_durations("Computation", comp_times));
    println!("{}", analyze_durations("Sent I/O", sent_io_times));
    println!("{}", analyze_durations("Recv I/O", recv_io_times));
    println!("{}", analyze_durations("Yield", yield_times));
    println!(
        "========================================================================================\n"
    );
}
