use fastly::log::Endpoint;
use std::sync::Mutex;
use tracing_fastly::{
    CorrelationLayer, StructuredEvent, StructuredEventSink,
    serialize::{self, datadog},
};
use tracing_subscriber::prelude::*;

fn setup_logging(service_name: &str) {
    let structured_layer = Endpoint::try_from_name("trace_logs").ok().map(|endpoint| {
        CorrelationLayer::new(TraceSink {
            service_name: service_name.to_owned(),
            endpoint: Mutex::new(endpoint),
        })
        .correlate("request_id")
    });

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact().with_ansi(false))
        .with(structured_layer)
        .init();
}

struct TraceSink {
    service_name: String,
    endpoint: Mutex<Endpoint>,
}

impl StructuredEventSink for TraceSink {
    fn emit(&self, event: &StructuredEvent<'_>) {
        let mut fields = event.fields.clone();
        if let Some(request_id) = event.correlation.get("request_id") {
            fields.insert("request_id".to_owned(), request_id.into());
        }

        let row = datadog::TraceLog {
            ddsource: "fastly",
            ddtags: datadog::Tags::new().with("env", "production"),
            hostname: fastly::compute_runtime::hostname(),
            timestamp: event.timestamp,
            message: event.message.to_owned(),
            service: &self.service_name,
            status: event.level,
            fields,
        };
        if let Err(error) = serialize::write_ndjson_row(&self.endpoint, &row) {
            eprintln!("failed to write structured trace event: {error}");
        }
    }
}

fn main() {
    setup_logging("example_service");

    let span = tracing::info_span!("request", request_id = tracing::field::Empty);
    span.record("request_id", "req-abc-123");
    let _guard = span.enter();

    tracing::info!(status = 200, backend = "origin", "handled request");
    tracing::warn!(reason = "stale", "cache miss");
}
