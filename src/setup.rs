//! One-call setup for the common cases.
//!
//! The structured [`CorrelationLayer`](crate::CorrelationLayer) + BigQuery
//! path (see `examples/bigquery.rs`) is the full-power option. Most services
//! want something far simpler: compact, human-readable lines that leave the
//! edge with no row schema to maintain. That's what this module is for.
//!
//! # Two destinations
//!
//! - [`init_stdout`] — write to **stdout**. On Fastly Compute the platform
//!   captures stdout and forwards it to whichever logging endpoint you've
//!   configured as the service's stdout destination. Nothing in code names an
//!   endpoint; it's a service-config setting. This is the easiest path.
//! - [`init_endpoint`] / [`EndpointWriter`] — write formatted lines straight
//!   to a **named Fastly endpoint** from code. This is the direct analog of
//!   [`log_fastly::Builder::default_endpoint`], but for `tracing`'s `fmt`
//!   output instead of the `log` façade.
//!
//! Both default the level to `INFO`, overridable via the `RUST_LOG`
//! environment variable. For anything more (custom format, multiple layers,
//! the BigQuery sink) compose the layers yourself — [`compact_stdout_layer`]
//! and [`compact_endpoint_layer`] are exposed for exactly that.
//!
//! [`log_fastly::Builder::default_endpoint`]:
//! https://docs.rs/log-fastly/latest/log_fastly/struct.Builder.html

use fastly::log::Endpoint;
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

/// A compact `fmt` layer writing to stdout — the building block behind
/// [`init_stdout`]. Exposed so you can stack it with other layers (e.g. the
/// BigQuery sink) in your own `registry()` chain.
pub fn compact_stdout_layer<S>() -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fmt::layer().compact()
}

/// A compact `fmt` layer writing formatted lines to the named Fastly log
/// endpoint — the building block behind [`init_endpoint`]. ANSI is disabled
/// since the destination isn't a terminal.
pub fn compact_endpoint_layer<S>(endpoint: &'static str) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fmt::layer()
        .compact()
        .with_ansi(false)
        .with_writer(EndpointWriter::new(endpoint))
}

/// A [`MakeWriter`] that sends each formatted log line to a named Fastly log
/// endpoint. Writing to an endpoint the service doesn't define is silently
/// dropped at the edge, so naming a missing endpoint is harmless.
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

/// Install a global subscriber writing compact lines to **stdout**.
///
/// The simplest setup: on Compute, stdout is forwarded to the logging endpoint
/// configured as the service's stdout destination, so this one call is enough
/// to get logs off the edge. Level defaults to `INFO` (`RUST_LOG` overrides).
///
/// Panics if a global subscriber is already installed — call once, early in
/// `main`. Use [`try_init_stdout`] to handle that case yourself.
pub fn init_stdout() {
    try_init_stdout().expect("a global tracing subscriber is already installed");
}

/// Fallible [`init_stdout`]: returns `Err` instead of panicking when a global
/// subscriber is already set.
pub fn try_init_stdout() -> Result<(), TryInitError> {
    tracing_subscriber::registry()
        .with(env_filter())
        .with(compact_stdout_layer())
        .try_init()
}

/// Install a global subscriber writing compact lines straight to the named
/// Fastly log **endpoint** (rather than relying on stdout capture). The
/// `tracing` analog of `log_fastly`'s `default_endpoint`.
///
/// Panics if a global subscriber is already installed; see
/// [`try_init_endpoint`].
pub fn init_endpoint(endpoint: &'static str) {
    try_init_endpoint(endpoint).expect("a global tracing subscriber is already installed");
}

/// Fallible [`init_endpoint`].
pub fn try_init_endpoint(endpoint: &'static str) -> Result<(), TryInitError> {
    tracing_subscriber::registry()
        .with(env_filter())
        .with(compact_endpoint_layer(endpoint))
        .try_init()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::{info, info_span, subscriber::with_default};

    // Global init can only run once per process, so exercise the layers with a
    // scoped (thread-local) subscriber instead. These confirm the layers build
    // and emit without panicking; under viceroy the writes are real.

    #[test]
    fn stdout_layer_emits() {
        let subscriber = tracing_subscriber::registry().with(compact_stdout_layer());
        with_default(subscriber, || {
            let span = info_span!("req", request_id = "abc");
            let _g = span.enter();
            info!(status = 200, "handled");
        });
    }

    #[test]
    fn endpoint_layer_emits() {
        let subscriber =
            tracing_subscriber::registry().with(compact_endpoint_layer("test_endpoint"));
        with_default(subscriber, || {
            info!("to the endpoint");
        });
    }
}
