// cli.rs — Command-line interface definition (clap v3 syntax).
//
// Flow: main() calls Cli::parse() → reads argv → returns a Cli struct.

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(name = "warpbench", about = "Distributed async HTTP load tester")]
pub struct Cli {
    /// Path to the YAML scenario config file
    #[clap(short, long, default_value = "scenario.yaml")]
    pub config: String,

    /// How many concurrent virtual users to simulate
    #[clap(short, long, default_value_t = 10)]
    pub users: usize,

    /// Total duration of the test in seconds
    #[clap(short, long, default_value_t = 10)]
    pub duration: u64,
}
