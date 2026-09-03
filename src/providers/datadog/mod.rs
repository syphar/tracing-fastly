//! A sink for sending structured logs to Datadog through a Fastly endpoint.
//!
//! The serialized records use Datadog's reserved `ddsource`, `ddtags`,
//! `hostname`, `message`, `service`, `status`, and `timestamp` attributes.
//! Effective tracing fields are nested under `fields`.

mod model;

use crate::{StructuredEvent, StructuredEventSink, serialize::NdjsonWriter};
use model::TraceLog;
use std::io::Write;

/// Writes structured tracing events to a Fastly Datadog logging endpoint.
pub struct TraceSink<W> {
    writer: NdjsonWriter<W>,
    source: String,
    tags: String,
    service: String,
    hostname: String,
}

impl<W> TraceSink<W> {
    /// Creates a Datadog sink writing to `writer` for the given service.
    ///
    /// The source defaults to `fastly`, tags default to an empty string, and
    /// the hostname is read from the Fastly Compute runtime.
    pub fn new(writer: W, service: impl Into<String>) -> Self {
        Self {
            writer: NdjsonWriter::new(writer),
            source: "fastly".to_owned(),
            tags: String::new(),
            service: service.into(),
            hostname: fastly::compute_runtime::hostname().to_owned(),
        }
    }

    /// Overrides the value of Datadog's reserved `ddsource` attribute.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Sets Datadog tags in its comma-separated `key:value` wire format.
    pub fn with_tags(mut self, tags: impl Into<String>) -> Self {
        self.tags = tags.into();
        self
    }
}

impl<W> StructuredEventSink for TraceSink<W>
where
    W: Write + Send + 'static,
{
    fn emit(&self, event: &StructuredEvent<'_>) {
        let row = TraceLog {
            ddsource: &self.source,
            ddtags: &self.tags,
            hostname: &self.hostname,
            timestamp: event.timestamp(),
            message: event.message(),
            service: &self.service,
            status: event.level(),
            fields: event.fields(),
        };

        if let Err(error) = self.writer.write(&row) {
            eprintln!("failed to write Datadog trace log: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::RecordWriter;
    use serde_json::{Map, Value, json};
    use std::time::UNIX_EPOCH;
    use tracing::Level;

    #[test]
    fn sink_emits_each_json_record_with_one_write() {
        let writer = RecordWriter::default();
        let records = writer.clone();
        let sink = TraceSink {
            writer: NdjsonWriter::new(writer),
            source: "fastly".to_owned(),
            tags: "env:production".to_owned(),
            service: "service".to_owned(),
            hostname: "host".to_owned(),
        };
        let fields = Map::from_iter([("request_id".to_owned(), json!("req-123"))]);

        sink.emit(&StructuredEvent {
            timestamp: UNIX_EPOCH,
            level: Level::INFO,
            message: "hello",
            fields: &fields,
        });

        let records = records.records().unwrap();
        assert_eq!(records.len(), 1);
        assert!(!records[0].ends_with(b"\n"));
        assert_eq!(
            serde_json::from_slice::<Value>(&records[0]).unwrap()["message"],
            "hello"
        );
    }
}
