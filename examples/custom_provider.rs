//! Implementing a provider-specific JSON format outside this crate.

use fastly::log::Endpoint;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::{io::Write, time::SystemTime};
use tracing::Level;
use tracing_fastly::{
    StructuredEvent, StructuredEventLayer, StructuredEventSink, serialize::NdjsonWriter,
};
use tracing_subscriber::{filter::LevelFilter, prelude::*};

/// The exact wire format expected by this example provider.
#[derive(Serialize)]
struct CustomLog<'a> {
    #[serde(serialize_with = "tracing_fastly::serialize::system_time::ser_unix_milliseconds")]
    occurred_at_ms: SystemTime,
    #[serde(serialize_with = "serialize_level")]
    severity: Level,
    body: &'a str,
    attributes: &'a Map<String, Value>,
}

struct CustomSink<W> {
    writer: NdjsonWriter<W>,
}

impl<W> CustomSink<W> {
    fn new(writer: W) -> Self {
        Self {
            writer: NdjsonWriter::new(writer),
        }
    }
}

fn serialize_level<S>(level: &Level, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(level.as_str())
}

impl<W> StructuredEventSink for CustomSink<W>
where
    W: Write + Send + 'static,
{
    fn emit(&self, event: &StructuredEvent<'_>) {
        let row = CustomLog {
            occurred_at_ms: event.timestamp(),
            severity: event.level(),
            body: event.message(),
            // This already contains both event fields and inherited span fields.
            attributes: event.fields(),
        };

        if let Err(error) = self.writer.write(&row) {
            eprintln!("failed to write custom log: {error}");
        }
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

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use serde_json::json;
    use tracing::subscriber::with_default;
    use tracing_fastly::testing::RecordWriter;

    #[test]
    fn writes_the_provider_format() {
        let writer = RecordWriter::default();
        let records = writer.clone();
        let subscriber =
            tracing_subscriber::registry().with(StructuredEventLayer::new(CustomSink::new(writer)));

        with_default(subscriber, || {
            let request = tracing::info_span!("request", request_id = "req-123");
            let _guard = request.enter();
            tracing::info!(status = 200, "handled request");
        });

        let records = records.records().unwrap();
        assert_eq!(records.len(), 1);
        let row: Value = serde_json::from_slice(&records[0]).unwrap();
        assert_eq!(row["body"], json!("handled request"));
        assert_eq!(row["severity"], json!("INFO"));
        assert_eq!(row["attributes"]["request_id"], json!("req-123"));
        assert_eq!(row["attributes"]["status"], json!(200));
    }
}
