mod model;
mod tags;

pub use model::TraceLog;
pub use tags::Tags;

use crate::{StructuredEvent, StructuredEventSink, serialize};
use fastly::log::Endpoint;
use serde::Serializer;
use std::sync::Mutex;
use tracing::Level;

/// Writes structured tracing events to a Fastly Datadog logging endpoint.
pub struct TraceSink {
    endpoint: Mutex<Endpoint>,
    source: String,
    tags: Tags,
    service: String,
}

impl TraceSink {
    pub fn new(endpoint: Endpoint, service: impl Into<String>) -> Self {
        Self {
            endpoint: Mutex::new(endpoint),
            source: "fastly".to_owned(),
            tags: Tags::new(),
            service: service.into(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    pub fn with_tags(mut self, tags: Tags) -> Self {
        self.tags = tags;
        self
    }
}

impl StructuredEventSink for TraceSink {
    fn emit(&self, event: &StructuredEvent<'_>) {
        let row = TraceLog {
            ddsource: &self.source,
            ddtags: &self.tags,
            hostname: fastly::compute_runtime::hostname(),
            timestamp: event.timestamp,
            message: event.message,
            service: &self.service,
            status: event.level,
            fields: event.fields,
        };

        if let Err(error) = serialize::write_ndjson_row(&self.endpoint, &row) {
            eprintln!("failed to write Datadog trace log: {error}");
        }
    }
}

/// serialize a `tracing::Level` with datadog convention:
/// * lower-case
/// * trace becomes debug
fn ser_tracing_level<S>(level: &Level, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let status = match *level {
        Level::ERROR => "error",
        Level::WARN => "warn",
        Level::INFO => "info",
        Level::DEBUG | Level::TRACE => "debug",
    };
    serializer.serialize_str(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value, json};
    use std::time::UNIX_EPOCH;

    #[test]
    fn tracing_levels_serialize_as_datadog_statuses() {
        fn serialized_status(level: Level) -> Value {
            serde_json::to_value(TraceLog {
                ddsource: "fastly",
                ddtags: &Tags::new(),
                hostname: "host",
                timestamp: UNIX_EPOCH,
                message: "",
                service: "service",
                status: level,
                fields: &Map::new(),
            })
            .unwrap()["status"]
                .clone()
        }

        assert_eq!(serialized_status(Level::ERROR), json!("error"));
        assert_eq!(serialized_status(Level::WARN), json!("warn"));
        assert_eq!(serialized_status(Level::INFO), json!("info"));
        assert_eq!(serialized_status(Level::DEBUG), json!("debug"));
        assert_eq!(serialized_status(Level::TRACE), json!("debug"));
    }
}
