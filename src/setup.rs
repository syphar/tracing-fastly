//! One-call setup for the common cases.
//!
//! The structured [`CorrelationLayer`](crate::CorrelationLayer) + BigQuery
//! path (see `examples/bigquery.rs`) is the full-power option. Most services
//! want something far simpler: compact, human-readable lines that leave the
//! edge with no row schema to maintain. That's what this module is for.
//!
//! # One format, choose the writer
//!
//! There is a single log *format* — [`compact_layer`], one compact `fmt`
//! layer. Where its lines go is a property of the **writer** you give it, not
//! a different layer:
//!
//! - stdout — [`init_stdout`]. On Fastly Compute the platform captures stdout
//!   and forwards it to whichever logging endpoint you've configured as the
//!   service's stdout destination; no endpoint is named in code.
//! - a named Fastly endpoint — [`init_endpoint`], via [`EndpointWriter`]. The
//!   `tracing` analog of [`log_fastly::Builder::default_endpoint`].
//! - **both at once** — tee the writers with
//!   [`MakeWriterExt::and`](tracing_subscriber::fmt::writer::MakeWriterExt::and):
//!
//! ```ignore
//! use tracing_subscriber::fmt::writer::MakeWriterExt;
//! use tracing_fastly::setup::EndpointWriter;
//!
//! // One layer, one format, teed to stdout *and* the "my_logs" endpoint.
//! tracing_fastly::setup::init(std::io::stdout.and(EndpointWriter::new("my_logs")));
//! ```
//!
//! All inits default the level to `INFO`, overridable via the `RUST_LOG`
//! environment variable. ANSI is disabled so the same bytes are valid whether
//! they land in a terminal or a log file.
//!
//! [`log_fastly::Builder::default_endpoint`]:
//! https://docs.rs/log-fastly/latest/log_fastly/struct.Builder.html

use fastly::log::Endpoint;
use std::io;
use tracing::Subscriber;
use tracing_subscriber::{
    EnvFilter,
    filter::LevelFilter,
    fmt::{self, MakeWriter},
    layer::Layer,
    prelude::*,
    registry::LookupSpan,
    util::TryInitError,
};

/// `INFO` by default, overridable through `RUST_LOG` (e.g.
/// `RUST_LOG=debug,my_module=trace`). `from_env_lossy` never fails — an
/// absent or malformed var just leaves the default in place.
fn env_filter() -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy()
}

/// The one log format: a compact, non-ANSI `fmt` layer writing to `writer`.
///
/// Point it at any [`MakeWriter`] — [`io::stdout`], an [`EndpointWriter`], or a
/// tee of several via
/// [`MakeWriterExt::and`](tracing_subscriber::fmt::writer::MakeWriterExt::and).
/// Exposed so you can stack it with other layers (e.g. the BigQuery sink) in
/// your own `registry()` chain.
pub fn compact_layer<S, W>(writer: W) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    fmt::layer().compact().with_ansi(false).with_writer(writer)
}

/// A [`MakeWriter`] that sends each formatted log line to a named Fastly log
/// endpoint. Composes like any other writer — tee it with
/// [`MakeWriterExt::and`](tracing_subscriber::fmt::writer::MakeWriterExt::and).
/// Writing to an endpoint the service doesn't define is silently dropped at
/// the edge, so naming a missing endpoint is harmless.
#[derive(Clone, Copy, Debug)]
pub struct EndpointWriter {
    endpoint: &'static str,
}

impl EndpointWriter {
    pub fn new(endpoint: &'static str) -> Self {
        Self { endpoint }
    }
}

impl<'a> MakeWriter<'a> for EndpointWriter {
    type Writer = Endpoint;
    fn make_writer(&'a self) -> Self::Writer {
        Endpoint::from_name(self.endpoint)
    }
}

/// Install a global subscriber that writes the compact format to `writer`
/// (`INFO` default, `RUST_LOG` override). The general entry point — pass
/// [`io::stdout`], an [`EndpointWriter`], or a tee of both.
///
/// Panics if a global subscriber is already installed; see [`try_init`].
pub fn init<W>(writer: W)
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    try_init(writer).expect("a global tracing subscriber is already installed");
}

/// Fallible [`init`]: returns `Err` instead of panicking when a global
/// subscriber is already set.
pub fn try_init<W>(writer: W) -> Result<(), TryInitError>
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    tracing_subscriber::registry()
        .with(env_filter())
        .with(compact_layer(writer))
        .try_init()
}

/// Shortcut for `init(io::stdout)` — the simplest setup. On Compute, stdout is
/// forwarded to the service's configured stdout endpoint.
pub fn init_stdout() {
    init(io::stdout);
}

/// Shortcut for `init(EndpointWriter::new(endpoint))` — write straight to a
/// named Fastly endpoint.
pub fn init_endpoint(endpoint: &'static str) {
    init(EndpointWriter::new(endpoint));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::{info, info_span, subscriber::with_default};
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    // Global init can only run once per process, so exercise the layer with a
    // scoped (thread-local) subscriber. These confirm the layer builds and
    // emits without panicking for each writer shape; under viceroy the writes
    // are real.

    fn emit() {
        let span = info_span!("req", request_id = "abc");
        let _g = span.enter();
        info!(status = 200, "handled");
    }

    #[test]
    fn to_stdout() {
        with_default(
            tracing_subscriber::registry().with(compact_layer(io::stdout)),
            emit,
        );
    }

    #[test]
    fn to_endpoint() {
        with_default(
            tracing_subscriber::registry().with(compact_layer(EndpointWriter::new("test_ep"))),
            emit,
        );
    }

    #[test]
    fn teed_to_both() {
        let writer = io::stdout.and(EndpointWriter::new("test_ep"));
        with_default(
            tracing_subscriber::registry().with(compact_layer(writer)),
            emit,
        );
    }
}
