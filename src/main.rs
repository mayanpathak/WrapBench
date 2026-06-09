// main.rs — Entry point. Glues everything together.
//
// Flow:
//   1. Parse CLI args  (cli.rs)
//   2. Init structured logging  (tracing)
//   3. Read YAML config file into a String buffer
//   4. Parse config from that buffer  (config.rs) — zero-copy, borrows from buffer
//   5. Hand off to async engine  (engine.rs)
//   6. Print report and exit

mod cli;
mod config;
mod engine;
mod error;
mod metrics;

use anyhow::Result;
use clap::Parser;
use tracing::info;

// `#[tokio::main]` transforms our async fn into a synchronous main() that
// spins up the tokio multi-threaded runtime and runs this function on it.
#[tokio::main]
async fn main() -> Result<()> {
    // Step 1: parse CLI args (--config, --users, --duration)
    let cli = cli::Cli::parse();

    // Step 2: set up structured logging.
    // RUST_LOG=debug warpbench  →  shows debug-level logs
    // RUST_LOG=info  warpbench  →  shows info and above (default)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    info!(
        "WarpBench starting: config={} users={} duration={}s",
        cli.config, cli.users, cli.duration
    );

    // Step 3: load YAML file into memory as one big String.
    // The lifetime 'config below must not outlive this buffer.
    let raw_config = std::fs::read_to_string(&cli.config)
        .map_err(|e| anyhow::anyhow!("Cannot read config '{}': {e}", cli.config))?;

    // Step 4: parse config — borrows &str slices from raw_config.
    let config = config::parse_config(&raw_config)?;

    if config.scenarios.is_empty() {
        anyhow::bail!("No scenarios found in config file.");
    }

    info!("Loaded {} scenario(s)", config.scenarios.len());

    // Step 5: run the load test engine.
    // raw_config must stay alive for the entire duration so the &str borrows
    // inside `config` remain valid — the compiler guarantees this.
    engine::run(&config.scenarios, cli.users, cli.duration).await?;

    Ok(())
}
