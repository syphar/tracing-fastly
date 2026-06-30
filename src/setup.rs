//! A [`MakeWriter`] for Fastly log endpoints, plus how to wire up a subscriber.
//!
//! There is intentionally no `init` helper. Wiring a subscriber is a few lines
//! of `tracing-subscriber`, and spelling it out keeps the format, level
//! filter, and destinations in your hands. The only piece this crate adds is
//! [`EndpointWriter`] — a writer that sends `fmt` output to a named Fastly
//! endpoint, the `tracing` analog of [`log_fastly`'s `default_endpoint`].
//!
//! The format lives in **one** `fmt` layer; the destination is just the
//! **writer** you give it. Use `.compact()` for terse lines and
//! `.with_ansi(false)` so the same bytes are valid in a terminal or a log
//! file, and an [`EnvFilter`] for levels (`INFO` default, `RUST_LOG` to
//! override).
//!
//! [`EnvFilter`]: tracing_subscriber::EnvFilter
//!
//! # Compact lines to stdout
//!
//! The simplest setup. On Fastly Compute the platform captures stdout and
//! forwards it to whichever logging endpoint you've configured as the
//! service's stdout destination — no endpoint is named in code.
//!
//! ```ignore
//! use tracing_subscriber::{EnvFilter, fmt, prelude::*};
//!
//! tracing_subscriber::registry()
//!     .with(EnvFilter::from_default_env())
//!     .with(fmt::layer().compact().with_ansi(false))
//!     .init();
//! ```
//!
//! # Straight to a named endpoint, or teed to both
//!
//! Point the same layer's writer at an [`EndpointWriter`], or tee it to stdout
//! *and* an endpoint with [`MakeWriterExt::and`] — one layer, one format, two
//! destinations:
//!
//! [`MakeWriterExt::and`]: tracing_subscriber::fmt::writer::MakeWriterExt::and
//!
//! ```ignore
//! use tracing_subscriber::{EnvFilter, fmt, fmt::writer::MakeWriterExt, prelude::*};
//! use tracing_fastly::EndpointWriter;
//!
//! let writer = std::io::stdout.and(EndpointWriter::new("my_logs"));
//! tracing_subscriber::registry()
//!     .with(EnvFilter::from_default_env())
//!     .with(fmt::layer().compact().with_ansi(false).with_writer(writer))
//!     .init();
//! ```
//!
//! # With the BigQuery sink
//!
//! The structured [`CorrelationLayer`](crate::CorrelationLayer) is just
//! another layer in the same chain — see `examples/bigquery.rs`.
//!
//! [`log_fastly`'s `default_endpoint`]:
//! https://docs.rs/log-fastly/latest/log_fastly/struct.Builder.html#method.default_endpoint

use fastly::log::Endpoint;
use tracing_subscriber::fmt::MakeWriter;

/// A [`MakeWriter`] that sends each formatted log line to a named Fastly log
/// endpoint.
///
/// Composes like any other writer — tee it with
/// [`MakeWriterExt::and`](tracing_subscriber::fmt::writer::MakeWriterExt::and)
/// to also write to stdout. Writing to an endpoint the service doesn't define
/// is silently dropped at the edge, so naming a missing endpoint is harmless.
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

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::{info, subscriber::with_default};
    use tracing_subscriber::{fmt, fmt::writer::MakeWriterExt, prelude::*};

    // A scoped (thread-local) subscriber, since a global one can only be set
    // once per process. Confirms EndpointWriter drives a fmt layer without
    // panicking, alone and teed with stdout; under viceroy the writes are real.

    #[test]
    fn writes_to_endpoint() {
        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().with_ansi(false).with_writer(EndpointWriter::new("test_ep")));
        with_default(subscriber, || info!("to the endpoint"));
    }

    #[test]
    fn tees_to_stdout_and_endpoint() {
        let writer = std::io::stdout.and(EndpointWriter::new("test_ep"));
        let subscriber =
            tracing_subscriber::registry().with(fmt::layer().with_ansi(false).with_writer(writer));
        with_default(subscriber, || info!("teed"));
    }
}
