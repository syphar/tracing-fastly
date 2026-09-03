use fastly::log::Endpoint;
use std::io;
use tracing_fastly::{StructuredEventLayer, providers::datadog};
use tracing_subscriber::prelude::*;

fn setup_logging(service_name: &str) {
    let endpoint = Endpoint::from_name("trace_logs");

    tracing_subscriber::registry()
        // log to stderr in compact format, for `fastly log-tail` and the
        // log-tailing UI in the fastly dashboard.
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_writer(io::stderr)
                .with_ansi(false),
        )
        .with(
            // `StructuredEventLayer` will package the span & event info into a
            // structured event that is easier to handle when we then
            // want to emit the log-record.
            StructuredEventLayer::new(
                // create the log-sink with datadog settings
                datadog::TraceSink::new(endpoint, service_name)
                    .with_tags(datadog::Tags::new().with("env", "production")),
            ),
        )
        .init();
}

fn main() {
    setup_logging("example_service");

    let span = tracing::info_span!("request", request_id = tracing::field::Empty);
    span.record("request_id", "req-abc-123");
    let _guard = span.enter();

    tracing::info!(status = 200, backend = "origin", "handled request");
    tracing::warn!(reason = "stale", "cache miss");
}
