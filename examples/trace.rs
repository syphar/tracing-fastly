use fastly::log::Endpoint;
use tracing_fastly::{
    CorrelationLayer,
    serialize::datadog::{Tags, TraceSink},
};
use tracing_subscriber::prelude::*;

fn setup_logging(service_name: &str) {
    let structured_layer = Endpoint::try_from_name("trace_logs").ok().map(|endpoint| {
        let sink =
            TraceSink::new(endpoint, service_name).with_tags(Tags::new().with("env", "production"));
        CorrelationLayer::new(sink).correlate("request_id")
    });

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact().with_ansi(false))
        .with(structured_layer)
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
