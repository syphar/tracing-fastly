use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Level;

/// A log entry using Datadog's reserved JSON attributes.
///
/// Additional event fields are flattened into the entry so they remain directly
/// searchable as Datadog attributes.
#[derive(Debug, Serialize)]
pub struct TraceLog<'a> {
    pub ddsource: &'a str,
    pub ddtags: &'a str,
    pub hostname: &'a str,
    #[serde(serialize_with = "ser_unix_milliseconds")]
    pub timestamp: SystemTime,
    pub message: String,
    pub service: &'a str,
    pub status: &'static str,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

/// Returns the Datadog status corresponding to a tracing level.
pub const fn level_name(level: Level) -> &'static str {
    match level {
        Level::ERROR => "error",
        Level::WARN => "warn",
        Level::INFO => "info",
        Level::DEBUG | Level::TRACE => "debug",
    }
}

fn ser_unix_milliseconds<S>(timestamp: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let milliseconds = timestamp
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    serializer.serialize_u128(milliseconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn trace_log_uses_datadog_reserved_fields_and_flattens_event_fields() {
        let row = TraceLog {
            ddsource: "fastly",
            ddtags: "env:production",
            hostname: "cache-fra1234",
            timestamp: UNIX_EPOCH + Duration::from_millis(1_500),
            message: "handled request".to_owned(),
            service: "docs.rs fastly WASM",
            status: level_name(Level::INFO),
            fields: [
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
                "backend": "origin",
                "http_status": 200,
            })
        );
    }

    #[test]
    fn tracing_levels_map_to_datadog_statuses() {
        assert_eq!(level_name(Level::ERROR), "error");
        assert_eq!(level_name(Level::WARN), "warn");
        assert_eq!(level_name(Level::INFO), "info");
        assert_eq!(level_name(Level::DEBUG), "debug");
        assert_eq!(level_name(Level::TRACE), "debug");
    }
}
