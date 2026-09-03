//! Implementing a provider-specific JSON format outside this crate.

use fastly::log::Endpoint;
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    io::Write,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::Level;
use tracing_fastly::{
    StructuredEvent, StructuredEventLayer, StructuredEventSink, serialize::write_ndjson_row,
};
use tracing_subscriber::{filter::LevelFilter, prelude::*};

/// The exact wire format expected by this example provider.
#[derive(Serialize)]
struct CustomLog<'a> {
    occurred_at_ms: u128,
    severity: &'static str,
    body: &'a str,
    attributes: &'a Map<String, Value>,
}

struct CustomSink<W> {
    writer: Mutex<W>,
}

impl<W> CustomSink<W> {
    fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
        }
    }
}

impl<W> StructuredEventSink for CustomSink<W>
where
    W: Write + Send + 'static,
{
    fn emit(&self, event: &StructuredEvent<'_>) {
        let row = CustomLog {
            occurred_at_ms: unix_milliseconds(event.timestamp()),
            severity: severity(event.level()),
            body: event.message(),
            // This already contains both event fields and inherited span fields.
            attributes: event.fields(),
        };

        let Ok(mut writer) = self.writer.lock() else {
            eprintln!("failed to lock custom log writer");
            return;
        };

        if let Err(error) = write_ndjson_row(&mut *writer, &row) {
            eprintln!("failed to write custom log: {error}");
        }
    }
}

fn unix_milliseconds(timestamp: SystemTime) -> u128 {
    timestamp
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn severity(level: Level) -> &'static str {
    match level {
        Level::ERROR => "error",
        Level::WARN => "warning",
        Level::INFO => "info",
        Level::DEBUG => "debug",
        Level::TRACE => "trace",
    }
}

fn main() {
    let endpoint = Endpoint::from_name("custom_logs");

    tracing_subscriber::registry()
        .with(StructuredEventLayer::new(CustomSink::new(endpoint)).with_filter(LevelFilter::INFO))
        .init();

    let request = tracing::info_span!("request", request_id = "req-abc-123", route = "/docs");
    let _guard = request.enter();

    tracing::info!(backend = "origin", status = 200, "handled request");
}
