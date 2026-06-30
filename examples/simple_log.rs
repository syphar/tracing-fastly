//! Simplest setup: compact `tracing` output, no BigQuery row schema.
//!
//! One `fmt` layer defines the format; its writer is teed so the same lines go
//! to **stdout** (for `fastly log-tail`) *and* a named Fastly log **endpoint**.
//! `fmt` takes a [`MakeWriter`](tracing_subscriber::fmt::MakeWriter) — a
//! per-line writer factory — and any closure `Fn() -> impl Write` is one, so a
//! `|| Endpoint::from_name(...)` closure is all that's needed. Writing to an
//! endpoint the service doesn't define is silently dropped at the edge, so
//! naming a missing one is harmless.
//!
//! Run `cargo build --example simple_log` to type-check; the Fastly hostcalls
//! only do anything inside the Compute runtime.

use fastly::log::Endpoint;
use std::io;
use tracing::{info, info_span, warn};
use tracing_subscriber::{
    EnvFilter, filter::LevelFilter, fmt, fmt::writer::MakeWriterExt, prelude::*,
};

const LOG_ENDPOINT: &str = "my_logs";

/// Install the global subscriber: one compact, non-ANSI format teed to stdout
/// and `LOG_ENDPOINT`. Level defaults to `INFO`, overridable via `RUST_LOG`.
fn setup_logging() {
    let writer = io::stdout.and(|| Endpoint::from_name(LOG_ENDPOINT));

    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with(fmt::layer().compact().with_ansi(false).with_writer(writer))
        .init();
}

fn main() {
    setup_logging();

    let _guard = info_span!("request", request_id = "req-abc-123").entered();
    info!(status = 200, backend = "origin", "handled request");
    warn!(reason = "stale", "cache miss");
}
