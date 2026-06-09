// error.rs — All custom error types for WarpBench.
//
// Flow: engine.rs creates HttpError values → sends them over the mpsc channel
//       → metrics collector counts failures by type.
//
// We use `thiserror` which auto-generates the Display + Error trait impls
// from the #[error("...")] annotation on each variant — no boilerplate needed.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum HttpError {
    // The network itself failed (DNS, TCP, TLS, timeout)
    #[error("Request failed: {0}")]
    Request(#[from] reqwest::Error),

    // Server replied but with a non-2xx status
    #[error("Bad status {status} for {url}")]
    BadStatus { status: u16, url: String },
}
