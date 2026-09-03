use crate::serialize::ser_unix_milliseconds;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use std::{fmt, time::SystemTime};
use tracing::Level;

/// A comma-separated collection of Datadog tags.
///
/// Datadog recommends `key:value` tags. Keys and values are kept verbatim;
/// callers should follow Datadog's tag naming requirements.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Tags(String);

impl Tags {
    pub const fn new() -> Self {
        Self(String::new())
    }

    /// Appends a `key:value` tag.
    pub fn push(&mut self, key: impl AsRef<str>, value: impl AsRef<str>) {
        self.push_separator();
        self.0.push_str(key.as_ref());
        self.0.push(':');
        self.0.push_str(value.as_ref());
    }

    /// Appends a tag without a value.
    pub fn push_bare(&mut self, tag: impl AsRef<str>) {
        self.push_separator();
        self.0.push_str(tag.as_ref());
    }

    pub fn with(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.push(key, value);
        self
    }

    pub fn with_bare(mut self, tag: impl AsRef<str>) -> Self {
        self.push_bare(tag);
        self
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn push_separator(&mut self) {
        if !self.0.is_empty() {
            self.0.push(',');
        }
    }
}

impl<K, V> FromIterator<(K, V)> for Tags
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut tags = Self::new();
        for (key, value) in iter {
            tags.push(key, value);
        }
        tags
    }
}

impl fmt::Display for Tags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A log entry using Datadog's reserved JSON attributes.
///
/// Additional event fields are flattened into the entry so they remain directly
/// searchable as Datadog attributes.
#[derive(Debug, Serialize)]
pub struct TraceLog<'a> {
    pub ddsource: &'a str,
    pub ddtags: Tags,
    pub hostname: &'a str,
    #[serde(serialize_with = "ser_unix_milliseconds")]
    pub timestamp: SystemTime,
    pub message: String,
    pub service: &'a str,
    #[serde(serialize_with = "ser_level")]
    pub status: Level,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

fn ser_level<S>(level: &Level, serializer: S) -> Result<S::Ok, S::Error>
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
    fn trace_log_uses_datadog_reserved_fields_and_flattens_event_fields() {
        let row = TraceLog {
            ddsource: "fastly",
            ddtags: Tags::new().with("env", "production"),
            hostname: "cache-fra1234",
            timestamp: UNIX_EPOCH + Duration::from_millis(1_500),
            message: "handled request".to_owned(),
            service: "docs.rs fastly WASM",
            status: Level::INFO,
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
    fn tracing_levels_serialize_as_datadog_statuses() {
        fn serialized_status(level: Level) -> Value {
            serde_json::to_value(TraceLog {
                ddsource: "fastly",
                ddtags: Tags::new(),
                hostname: "host",
                timestamp: UNIX_EPOCH,
                message: String::new(),
                service: "service",
                status: level,
                fields: Map::new(),
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

    #[test]
    fn tags_are_encoded_as_a_comma_separated_string() {
        let mut tags = Tags::new().with("env", "production");
        tags.push("version", "1.2.3");
        tags.push_bare("canary");

        assert_eq!(tags.as_str(), "env:production,version:1.2.3,canary");
        assert_eq!(
            serde_json::to_value(tags).unwrap(),
            json!("env:production,version:1.2.3,canary")
        );
    }
}
