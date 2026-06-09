// metrics/mod.rs — Real-time metrics collection.
//
// Flow:
//   1. Engine creates a MetricsSender and keeps a MetricsReceiver.
//   2. Each worker task gets a clone of MetricsSender (mpsc = Multi-Producer).
//   3. Workers send a Measurement after every request (duration + status).
//   4. A dedicated aggregator task drains the receiver and accumulates stats.
//   5. At the end, engine calls report() to print final numbers.
//
// Atomics (AtomicU64) are used for the counters that ALL workers touch
// simultaneously — they're lock-free, much faster than Mutex<u64>.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::mpsc;

/// One data point from a single HTTP request.
#[derive(Debug)]
pub struct Measurement {
    pub duration_ms: u64, // how long the request took
    pub status: u16,      // HTTP status code (200, 404, 500 …)
    pub success: bool,    // false if network error or bad status
}

/// Shared counters — wrapped in Arc so every worker can hold a reference.
/// AtomicU64 means multiple threads can increment without locking.
pub struct Counters {
    pub total: AtomicU64,
    pub success: AtomicU64,
    pub failed: AtomicU64,
}

impl Counters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            total: AtomicU64::new(0),
            success: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        })
    }
}

/// The sending half — cloned and given to each worker.
#[derive(Clone)]
pub struct MetricsSender(pub mpsc::Sender<Measurement>);

/// The receiving half — held by the aggregator task.
pub struct MetricsReceiver(pub mpsc::Receiver<Measurement>);

/// Create a linked sender/receiver pair (channel buffer = 10_000 messages).
pub fn channel() -> (MetricsSender, MetricsReceiver) {
    let (tx, rx) = mpsc::channel(10_000);
    (MetricsSender(tx), MetricsReceiver(rx))
}

/// Accumulate all latency data points into a Vec for percentile math.
/// This runs on a std::thread (CPU-bound) so it doesn't block tokio.
pub fn compute_report(mut latencies: Vec<u64>, counters: &Counters) {
    let total = counters.total.load(Ordering::Relaxed);
    let success = counters.success.load(Ordering::Relaxed);
    let failed = counters.failed.load(Ordering::Relaxed);

    if latencies.is_empty() {
        println!("\n=== WarpBench Report ===");
        println!("No requests completed.");
        return;
    }

    // Sort so we can slice percentiles — this is why we do it on a CPU thread
    latencies.sort_unstable();

    let len = latencies.len();
    let p50 = latencies[len * 50 / 100];
    let p95 = latencies[len * 95 / 100];
    let p99 = latencies[len * 99 / 100];
    let avg: u64 = latencies.iter().sum::<u64>() / len as u64;
    let min = latencies[0];
    let max = latencies[len - 1];

    println!("\n=== WarpBench Report ===");
    println!("Total requests : {total}");
    println!("Success        : {success}");
    println!("Failed         : {failed}");
    println!("Latency (ms)   : min={min}  avg={avg}  p50={p50}  p95={p95}  p99={p99}  max={max}");
}
