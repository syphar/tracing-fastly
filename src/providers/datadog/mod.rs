mod model;

use crate::{StructuredEvent, StructuredEventSink, serialize};
use model::TraceLog;
use std::{io::Write, sync::Mutex};

/// Writes structured tracing events to a Fastly Datadog logging endpoint.
pub struct TraceSink<W> {
    writer: Mutex<W>,
    source: String,
    tags: String,
    service: String,
    hostname: String,
}

impl<W> TraceSink<W> {
    pub fn new(writer: W, service: impl Into<String>) -> Self {
        Self {
            writer: Mutex::new(writer),
            source: "fastly".to_owned(),
            tags: String::new(),
            service: service.into(),
            hostname: fastly::compute_runtime::hostname().to_owned(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

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

        let Ok(mut writer) = self.writer.lock() else {
            eprintln!("failed to lock Datadog trace log writer");
            return;
        };
        if let Err(error) = serialize::write_ndjson_row(&mut *writer, &row) {
            eprintln!("failed to write Datadog trace log: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value, json};
    use std::{
        io,
        sync::{Arc, Mutex},
        time::UNIX_EPOCH,
    };
    use tracing::Level;

    #[derive(Clone, Default)]
    struct RecordWriter(Arc<Mutex<Vec<Vec<u8>>>>);

    impl Write for RecordWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().push(bytes.to_vec());
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn sink_emits_each_json_record_with_one_write() {
        let writer = RecordWriter::default();
        let records = Arc::clone(&writer.0);
        let sink = TraceSink {
            writer: Mutex::new(writer),
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

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert!(!records[0].ends_with(b"\n"));
        assert_eq!(
            serde_json::from_slice::<Value>(&records[0]).unwrap()["message"],
            "hello"
        );
    }
}
