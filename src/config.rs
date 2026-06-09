// config.rs — Zero-copy YAML configuration parsing.
//
// Flow: main() reads the YAML file into a String buffer → passes that buffer
//       to parse_config() → returns a Config<'_> whose string fields are
//       slices (&str) pointing into that same buffer — NO extra allocations.
//
// The lifetime 'a means: "this Config lives at most as long as the buffer
// it was parsed from." The compiler enforces this — you physically cannot
// drop the buffer while Config is still alive.
//
// #[serde(borrow)] is the magic that tells serde to borrow from the input
// instead of copying into a new String.

use anyhow::Result;
use serde::Deserialize;

/// One HTTP step inside a scenario (e.g. "POST /login", "GET /dashboard").
/// All string fields are &str — they point into the YAML buffer.
#[derive(Debug, Deserialize)]
pub struct Step<'a> {
    /// HTTP method: "GET", "POST", etc.
    #[serde(borrow)]
    pub method: &'a str,

    /// Full URL to request
    #[serde(borrow)]
    pub url: &'a str,

    /// Optional JSON body as a raw string
    #[serde(borrow, default)]
    pub body: Option<&'a str>,

    /// Optional list of "Key: Value" header strings
    #[serde(borrow, default)]
    pub headers: Vec<&'a str>,
}

/// A named scenario is a sequence of steps that each virtual user will replay.
#[derive(Debug, Deserialize)]
pub struct Scenario<'a> {
    #[serde(borrow)]
    pub name: &'a str,

    #[serde(borrow)]
    pub steps: Vec<Step<'a>>,
}

/// Top-level config — contains all scenarios.
#[derive(Debug, Deserialize)]
pub struct Config<'a> {
    #[serde(borrow)]
    pub scenarios: Vec<Scenario<'a>>,
}

/// Parse a YAML string (already loaded into memory) into a Config.
/// The returned Config borrows from `raw` — caller must keep `raw` alive.
pub fn parse_config(raw: &str) -> Result<Config<'_>> {
    let config = serde_yaml::from_str(raw)?;
    Ok(config)
}
