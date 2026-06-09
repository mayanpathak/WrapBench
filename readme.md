# WarpBench

> A distributed, async HTTP load-testing CLI written in Rust.
> Fire thousands of concurrent virtual users at your API — multi-step, session-aware, zero-copy.

---

## Table of Contents

1. [What Is WarpBench?](#1-what-is-warpbench)
2. [Why It Exists — The Design Philosophy](#2-why-it-exists--the-design-philosophy)
3. [What It Does](#3-what-it-does)
4. [How To Install & Run](#4-how-to-install--run)
5. [Writing a Scenario Config](#5-writing-a-scenario-config)
6. [Reading the Output](#6-reading-the-output)
7. [Full Architecture Diagram](#7-full-architecture-diagram)
8. [Data Flow — Step by Step](#8-data-flow--step-by-step)
9. [File-by-File Reference](#9-file-by-file-reference)
    - [Cargo.toml](#cargotoml)
    - [src/main.rs](#srcmainrs)
    - [src/cli.rs](#srccilrs)
    - [src/config.rs](#srcconfigrs)
    - [src/engine.rs](#srcenginrs)
    - [src/metrics/mod.rs](#srcmetricsmodrs)
    - [src/error.rs](#srcerrorrs)
    - [scenario.yaml](#scenarioyaml)
10. [Function-by-Function Reference](#10-function-by-function-reference)
11. [The Rust Concepts This Project Teaches](#11-the-rust-concepts-this-project-teaches)
    - [Lifetimes & Zero-Copy Parsing](#lifetimes--zero-copy-parsing)
    - [Async/Await & the Tokio Runtime](#asyncawait--the-tokio-runtime)
    - [mpsc Channels](#mpsc-channels)
    - [broadcast Channels](#broadcast-channels)
    - [Arc and Atomic Types](#arc-and-atomic-types)
    - [tokio::select!](#tokioselect)
    - [spawn_blocking](#spawn_blocking)
    - [Graceful Shutdown](#graceful-shutdown)
12. [Dependencies Explained](#12-dependencies-explained)
13. [Extending WarpBench](#13-extending-warpbench)
14. [Common Errors & Fixes](#14-common-errors--fixes)

---

## 1. What Is WarpBench?

WarpBench is a command-line HTTP load-testing tool — the same category as `wrk`, `vegeta`, or `k6`.

You point it at a YAML file that describes a sequence of HTTP requests (called a **scenario**), and it:

- Spawns **N concurrent virtual users**, each running your scenario in a loop
- Maintains **session state** (cookies, tokens) between steps of a scenario
- Collects **latency data** for every single request in real time
- Prints a **statistical report** at the end with P50 / P95 / P99 percentiles
- Handles **Ctrl+C gracefully** — finishing in-flight requests before exiting

The entire project is designed to demonstrate industry-grade Rust: zero-copy parsing, lock-free shared state, structured logging, graceful shutdown, and the tokio async runtime.

---

## 2. Why It Exists — The Design Philosophy

Most load testers just spam a single URL. Real APIs require multi-step flows: log in, fetch a token, use the token to call a protected endpoint, log out. WarpBench models that.

The project was also built to force you to *solve real problems* in Rust:

| Real Problem | WarpBench Solution |
|---|---|
| Parse huge configs without copying strings | Zero-copy parsing with `&'a str` and lifetimes |
| Share state between thousands of tasks | `Arc<AtomicU64>` — no locks in the hot path |
| Collect metrics without bottlenecking workers | `tokio::sync::mpsc` channel — lock-free streaming |
| Do CPU-heavy math without blocking the async runtime | `tokio::task::spawn_blocking` offloads to a thread pool |
| Tell all workers to stop at once | `tokio::sync::broadcast` — one sender, N receivers |
| Respond to both timer expiry AND Ctrl+C | `tokio::select!` — race two futures |

---

## 3. What It Does

```
You run:
  warpbench --users 50 --duration 30 --config scenario.yaml

WarpBench:
  1. Parses the YAML config (zero-copy)
  2. Spawns 50 async worker tasks
  3. Each worker loops: run all steps in the scenario → repeat
  4. After 30 seconds (or Ctrl+C), broadcasts shutdown
  5. Workers finish their current step and exit
  6. Prints report:

=== WarpBench Report ===
Total requests : 14293
Success        : 14201
Failed         : 92
Latency (ms)   : min=4  avg=103  p50=91  p95=248  p99=412  max=1038
```

---

## 4. How To Install & Run

### Prerequisites

- Rust toolchain (stable): https://rustup.rs
- Cargo (comes with Rust)

### Build

```bash
git clone <your-repo>
cd warpbench
cargo build --release
```

The optimized binary will be at `./target/release/warpbench`.

### Run

```bash
# Basic run — 10 users, 10 seconds, default scenario.yaml
./target/release/warpbench

# Custom options
./target/release/warpbench --users 100 --duration 60 --config my_scenario.yaml

# With debug logging
RUST_LOG=debug ./target/release/warpbench --users 5 --duration 5

# Short flags
./target/release/warpbench -u 50 -d 30 -c scenario.yaml
```

### CLI Flags

| Flag | Short | Default | Description |
|---|---|---|---|
| `--config` | `-c` | `scenario.yaml` | Path to the YAML scenario file |
| `--users` | `-u` | `10` | Number of concurrent virtual users |
| `--duration` | `-d` | `10` | Test duration in seconds |
| `--help` | `-h` | — | Print help text |

---

## 5. Writing a Scenario Config

A scenario is a YAML file containing a list of named scenarios. Each scenario is a list of HTTP steps. Every virtual user picks one scenario and runs it in a loop for the entire test duration.

```yaml
scenarios:
  - name: "login-then-dashboard"
    steps:
      # Step 1: POST login — server sets a session cookie
      - method: POST
        url: https://example.com/api/login
        body: '{"username": "test", "password": "test123"}'
        headers:
          - "Content-Type: application/json"
          - "Accept: application/json"

      # Step 2: GET dashboard — uses the cookie from step 1
      - method: GET
        url: https://example.com/api/dashboard

      # Step 3: PUT update profile
      - method: PUT
        url: https://example.com/api/profile
        body: '{"bio": "load tester"}'
```

### Rules

- `method` is case-insensitive. Supported: `GET`, `POST`, `PUT`, `DELETE`. Anything else defaults to `GET`.
- `body` is a raw string, sent verbatim. If body is present, `Content-Type: application/json` is added automatically (unless you override it in headers).
- `headers` is a list of strings in `"Key: Value"` format. The colon is the delimiter.
- Cookies set by the server in step N are automatically sent in step N+1 (same virtual user, same session).
- Multiple scenarios are supported. Virtual users are assigned round-robin: user 0 gets scenario 0, user 1 gets scenario 1, user 2 gets scenario 0 again, etc.

---

## 6. Reading the Output

```
=== WarpBench Report ===
Total requests : 14293       ← every HTTP call made across all users and all steps
Success        : 14201       ← 2xx responses
Failed         : 92          ← network errors + non-2xx responses
Latency (ms)   : min=4  avg=103  p50=91  p95=248  p99=412  max=1038
```

### Percentile Glossary

| Stat | Meaning |
|---|---|
| `min` | Fastest single request in the entire test |
| `avg` | Arithmetic mean of all latencies |
| `p50` | Median — 50% of requests were faster than this |
| `p95` | 95% of requests were faster than this. A key SLA metric. |
| `p99` | 99% of requests were faster than this. Shows tail latency. |
| `max` | Slowest single request — often an outlier |

P95 and P99 are the important ones. If your `avg` is 100ms but `p99` is 2000ms, 1% of your users wait 20× longer — that's a serious problem WarpBench will surface.

### Log Levels

Control log verbosity with the `RUST_LOG` environment variable:

```bash
RUST_LOG=error  # Only failures (quietest)
RUST_LOG=warn   # Failures + Ctrl+C events
RUST_LOG=info   # Normal operation (default)
RUST_LOG=debug  # Everything including per-request details
```

---

## 7. Full Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                          warpbench binary                           │
│                                                                     │
│  main()                                                             │
│    │                                                                │
│    ├─ Cli::parse()          reads --users --duration --config       │
│    │                                                                │
│    ├─ read_to_string()      loads YAML file into a String buffer    │
│    │                                                                │
│    ├─ parse_config()        zero-copy: Config borrows from buffer   │
│    │                                                                │
│    └─ engine::run()         ──────────────────────────────────────┐ │
│                                                                   │ │
│  engine::run()                                                    │ │
│    │                                                              │ │
│    ├─ reqwest::Client::new()   shared HTTP client + conn pool     │ │
│    │                                                              │ │
│    ├─ metrics::channel()       (MetricsSender, MetricsReceiver)   │ │
│    │                                                              │ │
│    ├─ Counters::new()          Arc<AtomicU64 × 3>                 │ │
│    │                                                              │ │
│    ├─ broadcast::channel()     shutdown signal                    │ │
│    │                                                              │ │
│    ├─ tokio::spawn() × N       N worker tasks ──────────────────┐ │ │
│    │                                                            │ │ │
│    │   Worker task (per virtual user)                           │ │ │
│    │     loop {                                                 │ │ │
│    │       tokio::select! {                                     │ │ │
│    │         shutdown_rx.recv() => break,        ◄──────────┐  │ │ │
│    │         run_steps(...) => continue,                     │  │ │ │
│    │       }                                                 │  │ │ │
│    │     }                                                   │  │ │ │
│    │                                                         │  │ │ │
│    │   run_steps() — for each step:                          │  │ │ │
│    │     execute_step()  → reqwest HTTP call ──► internet    │  │ │ │
│    │     measure latency                                     │  │ │ │
│    │     counters.total.fetch_add(1)  ◄── atomic, no lock   │  │ │ │
│    │     tx.send(Measurement)  ─────────────────────────────┼──┼─┤ │
│    │                                                         │  │ │ │
│    ├─ tokio::spawn() × 1       aggregator task ─────────────┼──┼─┘ │
│    │                                                         │  │   │
│    │   Aggregator                                            │  │   │
│    │     while let Some(m) = rx.recv() {                     │  │   │
│    │       latencies.push(m.duration_ms)  ◄── collecting     │  │   │
│    │     }                                                   │  │   │
│    │     return latencies                                    │  │   │
│    │                                                         │  │   │
│    ├─ tokio::select! {                                        │  │   │
│    │     sleep(duration) => { ... }                          │  │   │
│    │     ctrl_c()        => { ... }                          │  │   │
│    │   }                                                     │  │   │
│    │                                                         │  │   │
│    ├─ shutdown_tx.send(())  ─────────────────────────────────┘  │   │
│    │                                                            │   │
│    ├─ await all worker handles  ◄── join on all N tasks         │   │
│    │                                                            │   │
│    ├─ latencies = agg_handle.await  ◄── channel drained         │   │
│    │                                                            │   │
│    └─ spawn_blocking { compute_report(latencies, counters) }    │   │
│         ↓                                                       │   │
│       sort + percentiles (CPU work, off async runtime)          │   │
│       println! final report                                     │   │
└─────────────────────────────────────────────────────────────────────┘
```

### Channel Map

```
                    ┌────────────────────────────────────┐
                    │   mpsc channel (buffer: 10,000)    │
                    │                                    │
  Worker 0 ──tx──► │                                    │
  Worker 1 ──tx──► │  Measurement { duration_ms,        │ ──rx──► Aggregator
  Worker 2 ──tx──► │               status, success }    │
  ...              │                                    │
  Worker N ──tx──► │                                    │
                    └────────────────────────────────────┘

                    ┌────────────────────────────────────┐
                    │  broadcast channel (capacity: 1)   │
                    │                                    │
  Engine ──────────►│  shutdown signal ()                │──rx──► Worker 0
                    │                                    │──rx──► Worker 1
                    │                                    │──rx──► Worker 2
                    │                                    │──rx──► Worker N
                    └────────────────────────────────────┘
```

---

## 8. Data Flow — Step by Step

Here is the exact sequence of events from startup to report, in order:

```
1.  main() calls Cli::parse()
      └─ clap reads argv, fills Cli { config, users, duration }

2.  tracing_subscriber::fmt().init()
      └─ structured logging is now active; RUST_LOG controls level

3.  std::fs::read_to_string(cli.config)
      └─ entire YAML file loaded into one heap-allocated String (raw_config)

4.  config::parse_config(&raw_config)
      └─ serde_yaml deserializes the YAML
      └─ all string fields become &str slices pointing INTO raw_config
      └─ no heap allocations for individual strings — zero-copy
      └─ returns Config<'_> with lifetime tied to raw_config

5.  engine::run(&config.scenarios, users, duration)

6.    reqwest::Client::builder().cookie_store(true).build()
        └─ creates shared HTTP client with connection pool
        └─ cookie_store(true) means cookies persist per-client across requests

7.    metrics::channel()
        └─ creates mpsc channel with buffer capacity 10,000
        └─ returns (MetricsSender, MetricsReceiver)

8.    Counters::new()
        └─ creates Arc<Counters> with AtomicU64 for total/success/failed

9.    broadcast::channel::<()>(1)
        └─ creates shutdown signal channel

10.   for user_id in 0..users:
        └─ converts zero-copy &str steps into owned StepOwned (Strings)
            (required because tokio::spawn needs 'static futures)
        └─ clones: client, tx (MetricsSender), Arc<Counters>
        └─ subscribes to broadcast: shutdown_rx = shutdown_tx.subscribe()
        └─ tokio::spawn(async move { ... })
            └─ spawns worker task (non-blocking, registers with tokio scheduler)

11.   drop(tx)
        └─ engine drops its copy of the mpsc sender
        └─ now only worker tasks hold senders
        └─ when all workers exit (drop their tx), the channel closes automatically

12.   tokio::spawn(aggregator)
        └─ aggregator loops: while let Some(m) = rx.recv().await { ... }
        └─ pushes every Measurement's duration_ms into latencies: Vec<u64>
        └─ when channel closes (all tx dropped), loop ends, latencies returned

13.   [Workers running concurrently]
        Each worker:
          loop {
            tokio::select! {
              _ = shutdown_rx.recv()       => break  (exit loop)
              _ = run_steps(...)           => continue (loop again)
            }
          }

        run_steps():
          for each step in scenario:
            let start = Instant::now()
            execute_step() — builds reqwest request, fires it, awaits response
            let duration_ms = start.elapsed().as_millis()
            counters.total.fetch_add(1, Relaxed)  — atomic increment
            if success: counters.success.fetch_add(1, Relaxed)
            else:       counters.failed.fetch_add(1, Relaxed)
            tx.send(Measurement { duration_ms, status, success }).await

14.   tokio::select! (engine level)
        _ = tokio::time::sleep(duration_secs) => "Duration elapsed"
        _ = tokio::signal::ctrl_c()           => "Ctrl+C received"
      Whichever fires first wins.

15.   shutdown_tx.send(())
        └─ broadcast to all worker subscribers simultaneously
        └─ each worker's shutdown_rx.recv() unblocks → break → task exits

16.   for h in handles { h.await }
        └─ engine waits for every worker task to finish

17.   latencies = agg_handle.await
        └─ all tx senders now dropped → aggregator's rx returns None → loop ends
        └─ latencies Vec<u64> is returned from the task

18.   tokio::task::spawn_blocking(move || compute_report(latencies, &counters))
        └─ hands CPU-intensive work to a blocking thread pool
        └─ avoids blocking tokio's async scheduler
        └─ sorts latencies (O(n log n)) and calculates percentiles

19.   println! final report
        └─ total, success, failed, min/avg/p50/p95/p99/max latencies

20.   main() returns Ok(())
        └─ process exits cleanly
```

---

## 9. File-by-File Reference

### `Cargo.toml`

The manifest for the Rust project. Declares metadata and dependencies.

```toml
[package]
name = "warpbench"
version = "0.1.0"
edition = "2021"
```

**`edition = "2021"`** — uses Rust's 2021 edition rules (latest stable edition). Affects resolver behavior and some minor syntax details.

**Dependencies:**

| Crate | Purpose |
|---|---|
| `tokio` | The async runtime. Manages task scheduling, timers, I/O, signal handling. Feature `full` enables everything. |
| `reqwest` | HTTP client. Async, supports connection pooling, cookie jar, TLS. |
| `clap` | CLI argument parser. Derives a `Cli` struct from field annotations. |
| `serde` | Serialization/deserialization framework. The `derive` feature enables `#[derive(Deserialize)]`. |
| `serde_yaml` | YAML format plugin for serde. |
| `tracing` | Structured logging macros: `info!()`, `warn!()`, `error!()`, `debug!()`. |
| `tracing-subscriber` | Installs a log sink that formats and filters tracing events. |
| `thiserror` | Derive macro that auto-generates `Display` + `Error` impls for error enums. |
| `anyhow` | Flexible error type for application-level code. Used in `main()` and `engine::run()`. |

---

### `src/main.rs`

**Role:** Entry point. Owns the top-level lifetime anchor. Glues all modules together.

**Why it's important:** The `raw_config` String is declared here and lives until `main()` returns. This is intentional — the `Config<'_>` struct borrows `&str` slices from this buffer. As long as `raw_config` is alive, those borrows are valid. The Rust compiler enforces this: if you tried to move or drop `raw_config` before the engine finishes, it would be a compile error.

**Declarations:**
```rust
mod cli;      // src/cli.rs
mod config;   // src/config.rs
mod engine;   // src/engine.rs
mod error;    // src/error.rs
mod metrics;  // src/metrics/mod.rs
```

**`#[tokio::main]`** is a proc-macro that rewrites the function into:
```rust
fn main() {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { /* your code */ })
}
```
This starts the tokio multi-threaded scheduler (one OS thread per CPU core).

---

### `src/cli.rs`

**Role:** Defines command-line arguments using clap's derive API.

```rust
#[derive(Parser, Debug)]
#[clap(name = "warpbench", about = "Distributed async HTTP load tester")]
pub struct Cli {
    #[clap(short, long, default_value = "scenario.yaml")]
    pub config: String,

    #[clap(short, long, default_value_t = 10)]
    pub users: usize,

    #[clap(short, long, default_value_t = 10)]
    pub duration: u64,
}
```

**How it works:** `#[derive(Parser)]` makes clap read the struct fields and their annotations to build the argument parser. The doc-comment above each field becomes the `--help` description. `default_value_t` for integers uses the Rust `Display` trait to convert to a string for the help text.

`Cli::parse()` reads `std::env::args()`, fills the struct, and returns it — or prints usage and exits if args are invalid.

---

### `src/config.rs`

**Role:** Zero-copy YAML deserialization. The most educational file in the project for learning Rust lifetimes.

```rust
#[derive(Debug, Deserialize)]
pub struct Step<'a> {
    #[serde(borrow)]
    pub method: &'a str,
    #[serde(borrow)]
    pub url: &'a str,
    #[serde(borrow, default)]
    pub body: Option<&'a str>,
    #[serde(borrow, default)]
    pub headers: Vec<&'a str>,
}
```

**The `'a` lifetime parameter** means: "the `&str` fields in this struct borrow from some string that lives at least as long as `'a`." In practice, `'a` is the lifetime of the `raw_config` String in `main()`.

**`#[serde(borrow)]`** tells serde: "instead of allocating a new `String` for this field, borrow a `&str` slice from the input." This is the zero-copy mechanism.

**Why this matters for performance:** Without `#[serde(borrow)]`, serde would clone every URL, method name, and header string into a new heap allocation. With it, those strings are just pointers into the YAML buffer that was already read. For configs with thousands of URLs, this saves significant memory.

**`pub fn parse_config(raw: &str) -> Result<Config<'_>>`** — The `'_` lifetime is inferred from the `raw` parameter: "the returned Config borrows from `raw`, so Config cannot outlive `raw`."

---

### `src/engine.rs`

**Role:** The core of WarpBench. Spawns workers, manages channels, coordinates shutdown, offloads reporting.

This file has three public-facing items and three private helpers:

**`pub async fn run<'a>(scenarios, users, duration_secs)`**
The only public function. Called from `main()`. Does everything described in the Data Flow section above. Owns the setup, the waiting, and the teardown.

**`struct StepOwned`**
A private struct that mirrors `config::Step` but uses owned `String` instead of borrowed `&'a str`.

Why is this needed? `tokio::spawn` requires the async block to be `'static` — meaning it cannot hold any references to non-static data. `&'a str` is a reference with a finite lifetime, so it doesn't satisfy `'static`. The solution is to convert the borrowed steps into owned `StepOwned` structs at spawn time. This is the one allocation-per-test-start cost: each worker copies its scenario steps into owned Strings once, then reuses those for the entire test.

**`async fn run_steps(client, steps, tx, counters)`**
Loops through all steps in order. For each step, calls `execute_step`, measures wall-clock duration, updates atomic counters, and sends a `Measurement` to the aggregator. Never blocks — `execute_step` yields to tokio while waiting for the network.

**`async fn execute_step(client, step) -> Result<u16, HttpError>`**
Builds and fires a single HTTP request using `reqwest`. Parses headers from `"Key: Value"` format. Adds `Content-Type: application/json` if a body is present. Returns the HTTP status code on success, or an `HttpError` on failure.

---

### `src/metrics/mod.rs`

**Role:** All measurement types and real-time aggregation machinery.

**`struct Measurement`**
```rust
pub struct Measurement {
    pub duration_ms: u64,
    pub status: u16,
    pub success: bool,
}
```
One data point per HTTP request. Sent from workers to the aggregator via mpsc.

**`struct Counters`**
```rust
pub struct Counters {
    pub total: AtomicU64,
    pub success: AtomicU64,
    pub failed: AtomicU64,
}
```
Three lock-free counters. `AtomicU64::fetch_add(1, Ordering::Relaxed)` is a CPU-level atomic increment — faster than a mutex and never causes a thread to block. `Ordering::Relaxed` means we don't need memory ordering guarantees beyond "this increment eventually becomes visible" — which is sufficient for counters.

Wrapped in `Arc<Counters>` (Atomically Reference Counted) so every worker can hold a reference-counted pointer to the same struct. When the last Arc clone is dropped, the struct is freed.

**`struct MetricsSender(pub mpsc::Sender<Measurement>)`**
**`struct MetricsReceiver(pub mpsc::Receiver<Measurement>)`**
Thin newtypes around tokio's channel halves. `MetricsSender` derives `Clone`, which is needed because each worker gets its own clone.

**`pub fn channel() -> (MetricsSender, MetricsReceiver)`**
Factory function. Creates a channel with a buffer of 10,000 messages. If workers fill the buffer (produce faster than the aggregator consumes), `tx.send()` will briefly pause the sending worker — this is back-pressure. The buffer is large enough that this rarely happens in practice.

**`pub fn compute_report(mut latencies: Vec<u64>, counters: &Counters)`**
Called from `spawn_blocking()` on a dedicated OS thread.
1. Reads the atomic counters
2. `sort_unstable()` — faster than `sort()` for primitives, allows reordering equal elements
3. Slices percentiles using index arithmetic: `latencies[len * 99 / 100]` for P99
4. Prints the final report

---

### `src/error.rs`

**Role:** Custom error types using `thiserror`.

```rust
#[derive(Debug, Error)]
pub enum HttpError {
    #[error("Request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Bad status {status} for {url}")]
    BadStatus { status: u16, url: String },
}
```

**`#[derive(Error)]`** from `thiserror` auto-generates the `std::error::Error` trait impl.

**`#[error("...")]`** generates the `Display` impl. `{0}` refers to the first field; named fields use `{field_name}`.

**`#[from] reqwest::Error`** generates a `From<reqwest::Error> for HttpError` impl. This means `?` in `execute_step` automatically converts a `reqwest::Error` into `HttpError::Request(e)`.

Two variants:
- `Request` — network-level failure (DNS, TCP, TLS, timeout)
- `BadStatus` — server replied but with a non-2xx status code

---

### `scenario.yaml`

**Role:** The user-editable config file. The only file a user needs to change to test a different API.

```yaml
scenarios:
  - name: "httpbin-demo"
    steps:
      - method: GET
        url: https://httpbin.org/get
      - method: POST
        url: https://httpbin.org/post
        body: '{"user": "warpbench", "load": true}'
        headers:
          - "X-WarpBench-Run: 1"
```

All strings in this file will be zero-copy parsed — they're referenced directly from the in-memory YAML buffer, not copied.

---

## 10. Function-by-Function Reference

### `main::main() -> Result<()>`
Top-level entry point. Owns the raw config buffer. Calls `Cli::parse()`, initializes tracing, reads the YAML file, parses the config, calls `engine::run()`. The lifetime of `raw_config` here is what makes the zero-copy parsing safe — the compiler will refuse to compile if you try to let it drop too early.

---

### `cli::Cli::parse() -> Cli` (from clap)
Reads `std::env::args()`. Fills the struct. Exits with usage message on invalid input. No custom logic here — all clap auto-generated.

---

### `config::parse_config(raw: &str) -> Result<Config<'_>>`
Calls `serde_yaml::from_str(raw)`. Returns a `Config` whose string fields are `&str` slices pointing into `raw`. The `Result` is an `anyhow::Result` — if the YAML is malformed, returns an error with context.

---

### `engine::run<'a>(scenarios, users, duration_secs) -> Result<()>`
The orchestrator. Does everything:
1. Builds `reqwest::Client` (shared, connection-pooled)
2. Creates metrics channel and counters
3. Creates broadcast shutdown channel
4. Loops `0..users`: converts borrowed steps to owned, clones resources, spawns worker task
5. Drops engine's copy of `tx` (important: signals eventual channel closure)
6. Spawns aggregator task
7. `tokio::select!` waits for timer or Ctrl+C
8. Broadcasts shutdown
9. Awaits all worker handles
10. Awaits aggregator (collects latencies)
11. `spawn_blocking` → `compute_report`

---

### `engine::run_steps(client, steps, tx, counters)` — private
For each step in `steps`, calls `execute_step`, measures duration, updates counters, sends `Measurement`. Non-blocking: yields to tokio during each HTTP call.

---

### `engine::execute_step(client, step) -> Result<u16, HttpError>` — private
Builds a `reqwest::RequestBuilder` based on the step's method. Adds headers and body. Calls `.send().await`. Returns the status code or an `HttpError`.

The `.await` is the key point: while waiting for the remote server to respond, the tokio scheduler can run other tasks on the same thread. This is why thousands of concurrent workers don't need thousands of OS threads.

---

### `metrics::Counters::new() -> Arc<Counters>`
Creates an `Arc<Counters>` with all counters initialized to 0. The `Arc` wrapper allows multiple owners (each worker gets an `Arc::clone`, which increments the reference count without copying the data).

---

### `metrics::channel() -> (MetricsSender, MetricsReceiver)`
Creates a `tokio::sync::mpsc` channel with a buffer of 10,000. Returns wrapped sender/receiver. The sender can be cloned (given to each worker). The receiver is singular (held by the aggregator task).

---

### `metrics::compute_report(latencies: Vec<u64>, counters: &Counters)`
Reads counters with `Ordering::Relaxed`. Calls `sort_unstable()`. Computes percentiles by index. Prints everything. Runs on a blocking thread pool via `spawn_blocking`, so it never blocks tokio's async scheduler even if sorting millions of latency values.

---

## 11. The Rust Concepts This Project Teaches

### Lifetimes & Zero-Copy Parsing

**The problem:** You want to parse a YAML file into structs, but you don't want to clone every string field into a new heap allocation.

**The solution:** Read the entire file into a `String` buffer. Parse the YAML such that string fields in your structs are `&str` — pointers into that buffer. The lifetime annotation `'a` on your structs tells the compiler "these references borrow from something with lifetime `'a`."

```rust
struct Config<'a> {
    scenarios: Vec<Scenario<'a>>,
}
struct Scenario<'a> {
    name: &'a str,        // points into the YAML buffer
    steps: Vec<Step<'a>>,
}
```

The compiler enforces that `Config` cannot outlive the `String` it was parsed from. Violating this is impossible — it's a compile error, not a runtime error.

---

### Async/Await & the Tokio Runtime

**The problem:** Doing 10,000 HTTP requests concurrently without 10,000 OS threads.

**The solution:** Async/await. An `async fn` returns a `Future` — a suspended computation. `.await` yields control back to the scheduler while waiting for I/O. Tokio schedules thousands of futures on a small pool of OS threads (typically one per CPU core).

When `execute_step` calls `builder.send().await`, the worker task is suspended. Tokio runs other tasks on that thread while the network response arrives. When the response comes, tokio resumes the suspended task.

---

### mpsc Channels

**mpsc = Multi-Producer, Single-Consumer.**

```
Worker 0 ──tx.clone()──►
Worker 1 ──tx.clone()──► [channel buffer: 10,000] ──rx──► Aggregator
Worker N ──tx.clone()──►
```

Workers send `Measurement` values to a shared channel. One aggregator task reads from it. No locks — the channel is the synchronization primitive. When all senders (`tx` clones) are dropped, `rx.recv()` returns `None`, signaling the channel is done.

This pattern decouples producers (workers) from the consumer (aggregator). Workers never wait for the aggregator; they just send and move on (or briefly back-pressure if the buffer fills).

---

### broadcast Channels

**1 sender, N receivers. Every receiver gets every message.**

Used for the shutdown signal. The engine sends `()` once. All N worker tasks, each holding a `shutdown_rx`, receive it. This is how you tell all workers to stop simultaneously without a `Mutex<bool>`.

```rust
let (shutdown_tx, _) = broadcast::channel::<()>(1);
// ...
let mut shutdown_rx = shutdown_tx.subscribe(); // in each worker
// ...
shutdown_tx.send(()); // in engine — all workers receive this
```

---

### Arc and Atomic Types

**`Arc<T>`** — Atomically Reference Counted smart pointer. Allows multiple owners of the same data. When the last `Arc` clone is dropped, the data is freed. Safe to share across threads.

**`AtomicU64`** — An integer type that can be incremented, decremented, or swapped without a mutex. Uses CPU-level atomic instructions. `fetch_add(1, Ordering::Relaxed)` is typically a single machine instruction.

Used together: `Arc<Counters>` where `Counters` holds `AtomicU64` fields. Every worker gets an `Arc::clone` (cheap — just increments a reference count). Every worker calls `fetch_add` on the counters. No lock, no contention, no blocking.

---

### `tokio::select!`

Races multiple async operations and runs the branch that completes first. Cancels the other branches.

Used in two places:

**In each worker:**
```rust
tokio::select! {
    _ = shutdown_rx.recv() => break,        // if shutdown arrives first, exit
    _ = run_steps(...) => {}                // if scenario finishes first, loop again
}
```

**In the engine:**
```rust
tokio::select! {
    _ = tokio::time::sleep(duration) => { /* timer expired */ }
    _ = tokio::signal::ctrl_c() => { /* user pressed Ctrl+C */ }
}
```

Without `select!`, you'd need complex coordination. With it, whichever async operation resolves first simply wins.

---

### `spawn_blocking`

The tokio runtime is designed for async I/O. It has a small number of threads. If you run CPU-intensive work on one of those threads (like sorting millions of latency values), you block the scheduler from running other tasks.

`tokio::task::spawn_blocking(closure)` sends the closure to a *separate* blocking thread pool — sized differently, designed for synchronous CPU work. The async caller `.await`s on a `JoinHandle`.

```rust
tokio::task::spawn_blocking(move || {
    // This runs on a blocking thread, not an async thread
    metrics::compute_report(latencies, &counters);
})
.await?;
```

**Rule of thumb:** Any CPU-bound work that takes more than ~100 microseconds should use `spawn_blocking` to avoid starving the async scheduler.

---

### Graceful Shutdown

The difference between a tool that works and one that's production-grade.

**Problem:** When the user presses Ctrl+C or the timer fires, you must:
1. Stop spawning new work
2. Let in-flight requests finish (don't kill mid-request)
3. Collect all metrics gathered so far
4. Print the report
5. Exit cleanly

**Solution:**
1. `tokio::signal::ctrl_c()` and `tokio::time::sleep()` — race with `select!`
2. `broadcast::send(())` — tells all workers to stop looping
3. Workers finish their current `run_steps()` call, then break on the next `select!`
4. `handles.await` — engine waits for every worker to actually exit
5. Aggregator drains remaining messages, returns `latencies`
6. `compute_report` prints

No `std::process::exit()`. No panicking. No orphaned in-flight requests.

---

## 12. Dependencies Explained

### `tokio = { version = "1", features = ["full"] }`

Tokio is Rust's most popular async runtime. It provides:
- `tokio::spawn()` — spawning async tasks
- `tokio::sync::mpsc` — multi-producer single-consumer channels
- `tokio::sync::broadcast` — one-to-many channels
- `tokio::time::sleep()` — non-blocking timers
- `tokio::signal::ctrl_c()` — OS signal handling
- `tokio::task::spawn_blocking()` — offloading to blocking threads
- `tokio::select!` — racing async operations

Feature `"full"` enables everything. In production you'd enable only what you need.

### `reqwest = { version = "0.11", features = ["json", "cookies"] }`

Async HTTP client built on top of tokio and hyper. Key features used:
- Connection pooling (the `Client` is shared across all workers)
- Cookie jar (`cookie_store(true)`) — cookies persist between requests on the same client
- Configurable timeout
- TLS support (rustls or native-tls)
- Supports GET, POST, PUT, DELETE, PATCH etc.

### `clap = { version = "3", features = ["derive"] }`

CLI argument parser. The `derive` feature lets you write a struct and derive the parser from it. Handles: positional args, flags with defaults, auto-generated help text, validation.

### `serde + serde_yaml`

Serde is Rust's serialization framework. `#[derive(Deserialize)]` generates deserialization code at compile time from your struct layout. `serde_yaml` is the YAML format adapter. `#[serde(borrow)]` enables zero-copy string borrowing.

### `tracing + tracing-subscriber`

`tracing` provides structured, leveled logging macros (`info!`, `warn!`, `error!`, `debug!`). Unlike `println!`, tracing records *structured* events with metadata (timestamp, module path, log level). `tracing-subscriber` installs a subscriber that formats these events and respects `RUST_LOG` filters.

Senior engineers use `tracing` instead of `println!` because:
- Log levels can be filtered at runtime without recompiling
- Structured logs can be sent to observability systems (Jaeger, Datadog, etc.)
- Logs include module context automatically

### `thiserror`

A derive macro that eliminates boilerplate for error types. Instead of:
```rust
impl std::fmt::Display for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // ...
    }
}
impl std::error::Error for MyError { /* ... */ }
```
You write:
```rust
#[derive(Error)]
#[error("description: {field}")]
struct MyError { field: String }
```

### `anyhow`

Flexible error type for application code. `anyhow::Result<T>` is short for `Result<T, anyhow::Error>`. The `anyhow::Error` type can hold any error that implements `std::error::Error`, plus contextual messages. Used in `main()` and `engine::run()`. `thiserror` is for library errors; `anyhow` is for application-level error propagation.

---

## 13. Extending WarpBench

### Add Bearer Token Auth

In `execute_step`, after building the request:
```rust
if let Some(token) = &step.token {
    builder = builder.bearer_auth(token);
}
```
Add `token: Option<String>` to `StepOwned` and `token: Option<&'a str>` to `config::Step`.

### Add Think Time (Delay Between Steps)

In `run_steps`, after sending the measurement:
```rust
if let Some(delay_ms) = step.delay_ms {
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
}
```

### Add Per-Second Request Rate (Rate Limiting)

Before `tokio::spawn` in the worker loop, use `tokio::time::interval()` to tick at a fixed rate instead of looping as fast as possible.

### Add HTTP/2 Support

```rust
let client = Client::builder()
    .http2_prior_knowledge()  // force HTTP/2
    .build()?;
```

### Add Real-Time Progress Output

Spawn a third task that reads from the atomic counters every second and prints progress:
```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let t = counters.total.load(Ordering::Relaxed);
        println!("Requests: {t}");
    }
});
```

### Add JSON Report Output

In `compute_report`, serialize the stats to JSON using `serde_json::json!({...})` and write to a file.

### Add Multiple Scenario Support with Weights

Instead of round-robin, assign scenarios to workers based on weights defined in the YAML:
```yaml
scenarios:
  - name: "login"
    weight: 70      # 70% of users run this
    steps: [...]
  - name: "browse"
    weight: 30      # 30% of users run this
    steps: [...]
```

---

## 14. Common Errors & Fixes

### `invalid peer certificate: UnknownIssuer`

**Cause:** Your system doesn't trust the server's TLS certificate, or you're running in a sandboxed environment with an intercepting proxy.

**Fix:**
```rust
// In engine.rs, add to Client::builder():
.danger_accept_invalid_certs(true)
// OR for self-signed certs:
.add_root_certificate(cert)
```

---

### `No scenarios found in config file`

**Cause:** YAML is valid but has no `scenarios` key, or the value is an empty list.

**Fix:** Check your YAML structure — the top level must have a `scenarios:` key with at least one entry.

---

### Workers exit immediately / `Error` on every request

**Cause:** The target URL is unreachable, or the response is non-2xx (WarpBench treats all non-2xx as failures).

**Fix:** Verify the URL is correct and reachable. Use `RUST_LOG=debug` to see the full error message.

---

### `error: package X requires rustc Y.Y.Y or newer`

**Cause:** Your Rust toolchain is too old for a dependency.

**Fix:** Update Rust: `rustup update stable`, or pin the dependency to an older version in `Cargo.toml` using `cargo update <crate> --precise <version>`.

---

### `cannot find attribute 'arg' in this scope` (clap version mismatch)

**Cause:** Using clap v4 attribute syntax (`#[arg(...)]`) with clap v3, or vice versa.

**Fix:** clap v3 uses `#[clap(...)]`. clap v4 uses `#[arg(...)]`. Match your syntax to your declared version in `Cargo.toml`.

---

### High `p99` but low `avg`

**Not a bug — this is the whole point.** P99 being much higher than avg indicates tail latency: most requests are fast, but some are slow. This could be GC pauses, connection queue buildup, or database lock contention on the server. WarpBench surfaces this; it's up to you to investigate.

---

### Channel buffer full / back-pressure

**Symptom:** Workers briefly pause between requests at very high concurrency.

**Cause:** The aggregator task is slower than workers at draining the metrics channel.

**Fix:** Increase the channel buffer in `metrics::channel()`:
```rust
let (tx, rx) = mpsc::channel(100_000); // was 10_000
```
Or, for extremely high throughput, consider using a lock-free queue or a histogram (HdrHistogram) instead of collecting raw latency values.

---

## Project Summary

WarpBench is ~250 lines of Rust across 6 files. In those 250 lines, it demonstrates:

- Lifetime-annotated zero-copy parsing
- Multi-threaded async concurrency with tokio
- Lock-free shared state with Arc + Atomics
- Channel-based metrics streaming (mpsc)
- One-to-many broadcast shutdown
- CPU/IO separation with spawn_blocking
- Graceful signal handling
- Professional structured logging

This is not beginner Rust. But by reading each file, understanding each lifetime annotation, and tracing the data flow from YAML buffer to final report, you gain a complete picture of how real systems-level Rust applications are built.
#