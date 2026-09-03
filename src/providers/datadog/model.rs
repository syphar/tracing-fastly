use crate::serialize;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::time::SystemTime;
use tracing::Level;

/// A log entry using Datadog's reserved JSON attributes.
///
/// Additional event fields are kept under `fields` to avoid collisions with
/// Datadog's reserved attributes.
#[derive(Debug, Serialize)]
pub struct TraceLog<'a> {
    pub ddsource: &'a str,
    pub ddtags: &'a str,
    pub hostname: &'a str,
    /// Milliseconds since the Unix epoch.
    /// Format / name:
    /// <https://docs.datadoghq.com/logs/log_configuration/processors/log_date_remapper/>
    #[serde(serialize_with = "serialize::system_time::ser_unix_milliseconds")]
    pub timestamp: SystemTime,
    pub message: &'a str,
    pub service: &'a str,
    #[serde(serialize_with = "ser_tracing_level")]
    pub status: Level,
    pub fields: &'a Map<String, Value>,
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
    use serde_json::json;
    use std::time::Duration;
    use std::time::UNIX_EPOCH;

    #[test]
    fn trace_log_uses_datadog_reserved_fields_and_nests_event_fields() {
        let row = TraceLog {
            ddsource: "fastly",
            ddtags: "env:production",
            hostname: "cache-fra1234",
            timestamp: UNIX_EPOCH + Duration::from_millis(1_500),
            message: "handled request",
            service: "docs.rs fastly WASM",
            status: Level::INFO,
            fields: &[
                ("backend".to_owned(), json!("origin")),
                ("http_status".to_owned(), json!(200)),
            ]
            .into_iter()
            .collect(),
        };

        assert_eq!(
            serde_json::to_value(row).unwrap(),
            json!({
                "ddsource": "fastly",
                "ddtags": "env:production",
                "hostname": "cache-fra1234",
                "timestamp": 1500,
                "message": "handled request",
                "service": "docs.rs fastly WASM",
                "status": "info",
                "fields": {
                    "backend": "origin",
                    "http_status": 200,
                },
            })
        );
    }

    #[test]
    fn tracing_levels_serialize_as_datadog_statuses() {
        fn serialized_status(level: Level) -> Value {
            serde_json::to_value(TraceLog {
                ddsource: "fastly",
                ddtags: "",
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
