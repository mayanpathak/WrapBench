// engine.rs — The heart of WarpBench.
//
// High-level flow:
//
//   run()
//    ├─ build a shared reqwest::Client (connection pool shared by all workers)
//    ├─ create metrics channel (tx cloned per worker, rx kept here)
//    ├─ create atomic counters (Arc cloned per worker)
//    ├─ create broadcast shutdown channel
//    ├─ spawn N worker tasks (one per virtual user)
//    ├─ spawn 1 aggregator task (drains the metrics channel)
//    ├─ wait for EITHER: test duration elapsed  OR  Ctrl+C
//    ├─ broadcast shutdown signal to all workers
//    ├─ await all worker handles
//    ├─ close metrics channel → aggregator task finishes
//    ├─ collect all latencies
//    └─ call compute_report() on a blocking thread (CPU math, not async)

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use reqwest::Client;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::config::Scenario;
use crate::error::HttpError;
use crate::metrics::{self, Counters, Measurement, MetricsSender};

/// Entry point called from main.
/// `scenarios` must outlive this function (they borrow from the YAML buffer).
pub async fn run<'a>(
    scenarios: &'a [Scenario<'a>],
    users: usize,
    duration_secs: u64,
) -> Result<()> {
    // --- Setup ---

    // One HTTP client for everyone — internally has a connection pool.
    // `cookie_store(true)` keeps cookies between steps (session state).
    let client = Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(10))
        .build()?;

    // Atomic counters shared by all workers.
    let counters = Counters::new();

    // mpsc channel: workers send Measurements, aggregator receives them.
    let (tx, mut rx) = metrics::channel();

    // broadcast channel: engine sends ONE shutdown signal, ALL workers receive it.
    // Capacity 1 is enough — we only ever send one signal.
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // --- Spawn workers ---

    info!("Starting {users} virtual users for {duration_secs}s");

    // Collect JoinHandles so we can await them all later.
    let mut handles = Vec::new();

    for user_id in 0..users {
        // Pick which scenario this user runs (round-robin if many scenarios).
        let scenario = &scenarios[user_id % scenarios.len()];

        // Clone everything the worker needs to own.
        let client = client.clone();
        let tx = tx.clone(); // MetricsSender is Clone
        let counters = Arc::clone(&counters);
        let mut shutdown_rx = shutdown_tx.subscribe(); // each worker gets its own receiver

        // Collect the steps into owned data so the worker task can be 'static.
        // (tokio::spawn requires 'static futures; we can't pass &'a str across.)
        let steps: Vec<StepOwned> = scenario
            .steps
            .iter()
            .map(|s| StepOwned {
                method: s.method.to_string(),
                url: s.url.to_string(),
                body: s.body.map(|b| b.to_string()),
                headers: s.headers.iter().map(|h| h.to_string()).collect(),
            })
            .collect();

        // Spawn the async task — this is non-blocking, it just registers the task.
        let handle = tokio::spawn(async move {
            // Loop: run the scenario over and over until we get a shutdown signal.
            loop {
                // tokio::select! polls multiple async branches simultaneously.
                // Whichever branch resolves first "wins" and the others are cancelled.
                tokio::select! {
                    // Branch 1: shutdown signal arrived → exit the loop.
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                    // Branch 2: run one full scenario pass (all steps).
                    // `biased` is NOT set, so tokio randomly picks which branch
                    // to check first — fair scheduling.
                    _ = run_steps(&client, &steps, &tx, &counters) => {
                        // scenario finished; loop back and start again
                    }
                }
            }
            info!("Worker {user_id} shut down");
        });

        handles.push(handle);
    }

    // Drop our copy of tx — when all workers also drop theirs, the channel closes
    // and the aggregator task knows to stop.
    drop(tx);

    // --- Aggregator task ---
    // Drains the mpsc channel and collects latencies into a Vec.
    // Using a tokio task here (not a thread) because it's just memory ops — fast.
    let agg_handle = tokio::spawn(async move {
        let mut latencies: Vec<u64> = Vec::new();
        // `recv()` returns None when all senders are dropped → clean shutdown.
        while let Some(m) = rx.0.recv().await {
            latencies.push(m.duration_ms);
        }
        latencies // return the collected data
    });

    // --- Wait for test to finish or user hits Ctrl+C ---
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(duration_secs)) => {
            info!("Duration elapsed, shutting down…");
        }
        _ = tokio::signal::ctrl_c() => {
            warn!("Ctrl+C received, shutting down gracefully…");
        }
    }

    // --- Graceful shutdown ---

    // Tell all workers to stop.  `send` returns Err if no subscribers — fine,
    // it means they already finished.
    let _ = shutdown_tx.send(());

    // Wait for every worker to exit cleanly.
    for h in handles {
        let _ = h.await;
    }

    // Aggregator will finish once all senders are gone (we dropped tx above and
    // workers dropped theirs when they exited).
    let latencies = agg_handle.await?;

    // --- Report ---
    // Run the heavy percentile sort on a blocking thread so we don't hold the
    // tokio scheduler during a potentially expensive operation.
    let counters_ref = Arc::clone(&counters);
    tokio::task::spawn_blocking(move || {
        metrics::compute_report(latencies, &counters_ref);
    })
    .await?;

    Ok(())
}

// ─── Owned step data (needed to satisfy 'static bound on tokio::spawn) ───────

/// Same as config::Step but with owned Strings instead of &str.
/// Workers hold this so they don't need the original YAML buffer to stay alive.
struct StepOwned {
    method: String,
    url: String,
    body: Option<String>,
    headers: Vec<String>, // each entry is "Header-Name: value"
}

// ─── Run one full pass through all steps in a scenario ────────────────────────

async fn run_steps(
    client: &Client,
    steps: &[StepOwned],
    tx: &MetricsSender,
    counters: &Counters,
) {
    use std::sync::atomic::Ordering;

    for step in steps {
        let start = Instant::now();
        let result = execute_step(client, step).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        // Update atomic counters — these are visible instantly to all workers.
        counters.total.fetch_add(1, Ordering::Relaxed);

        let (success, status) = match result {
            Ok(s) => {
                counters.success.fetch_add(1, Ordering::Relaxed);
                (true, s)
            }
            Err(ref e) => {
                counters.failed.fetch_add(1, Ordering::Relaxed);
                error!("Step failed: {e}");
                (false, 0)
            }
        };

        // Send the measurement to the aggregator.
        // If the channel is full or closed, just drop the measurement — never block.
        let _ = tx
            .0
            .send(Measurement {
                duration_ms,
                status,
                success,
            })
            .await;
    }
}

// ─── Execute a single HTTP step ───────────────────────────────────────────────

async fn execute_step(client: &Client, step: &StepOwned) -> Result<u16, HttpError> {
    // Build the request from method + url.
    let mut builder = match step.method.to_uppercase().as_str() {
        "POST" => client.post(&step.url),
        "PUT" => client.put(&step.url),
        "DELETE" => client.delete(&step.url),
        _ => client.get(&step.url), // default to GET
    };

    // Add headers (format: "Key: Value")
    for h in &step.headers {
        if let Some((k, v)) = h.split_once(':') {
            builder = builder.header(k.trim(), v.trim());
        }
    }

    // Add body if present
    if let Some(body) = &step.body {
        builder = builder
            .header("Content-Type", "application/json")
            .body(body.clone());
    }

    // Fire! (This yields to the tokio scheduler while waiting for the network.)
    let response = builder.send().await?;
    let status = response.status().as_u16();

    if !response.status().is_success() {
        return Err(HttpError::BadStatus {
            status,
            url: step.url.clone(),
        });
    }

    Ok(status)
}
