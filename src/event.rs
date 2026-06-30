//! The destination-agnostic shape of one logged event.
//!
//! [`CorrelationLayer`](crate::CorrelationLayer) builds a [`StructuredEvent`]
//! per `tracing` event and hands it to an [`EventSink`](crate::EventSink).
//! Nothing here knows about BigQuery, Fastly, or any particular row schema —
//! that mapping lives in the sink.

use serde_json::{Map, Value};
use std::time::SystemTime;
use tracing::{
    Level,
    field::{Field, Visit},
};

/// One `tracing` event, split into the two parts a structured sink needs.
///
/// Borrows everything from the layer's per-event scratch space, so it is only
/// valid for the duration of the [`EventSink::emit`](crate::EventSink::emit)
/// call.
pub struct StructuredEvent<'a> {
    /// Wall-clock time the event was observed (stamped by the layer).
    pub timestamp: SystemTime,
    /// The event's level (`INFO`, `WARN`, …).
    pub level: Level,
    /// The `message` — from `info!("hello {x}")`'s synthesized `message`
    /// field or an explicit `message = "..."` field (last write wins).
    pub message: &'a str,
    /// Every field that isn't `message`, as a JSON object. May be empty.
    pub fields: &'a Map<String, Value>,
    /// Span fields the layer was told to propagate (e.g. `request_id`),
    /// resolved from the event's span scope. See [`CorrelationFields`].
    pub correlation: &'a CorrelationFields,
}

impl StructuredEvent<'_> {
    /// The non-message fields as a payload, or `None` when there are none —
    /// convenient for an `Option` column that should be omitted when empty.
    pub fn payload(&self) -> Option<&Map<String, Value>> {
        (!self.fields.is_empty()).then_some(self.fields)
    }
}

/// Span-context fields propagated to an event, in capture order.
///
/// Produced by [`CorrelationLayer`](crate::CorrelationLayer) from the fields
/// it was configured to `correlate`. When the same field is present on
/// multiple ancestor spans the **outermost** (closest to the root) wins, which
/// matches typical request scoping: a per-request `request_id` set on the root
/// span flows to every nested span's events.
#[derive(Debug, Default, Clone)]
pub struct CorrelationFields {
    // Small and append-only; a Vec beats a map for the handful of fields a
    // sink ever correlates on, and preserves capture order.
    pub(crate) values: Vec<(&'static str, String)>,
}

impl CorrelationFields {
    /// The resolved value for `name`, if it was captured.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Iterate the captured `(field, value)` pairs in capture order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &str)> + '_ {
        self.values.iter().map(|(k, v)| (*k, v.as_str()))
    }

    /// Whether no correlation fields were resolved for this event.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Splits one event's fields into a `message` string and a `fields` map.
///
/// The `record_*` methods funnel through [`Self::insert`], which holds the
/// only non-trivial behaviour: the message/payload split, last-write-wins on
/// `message`, and coercion of a non-string `message` to text. `f64` NaN/Inf
/// are dropped (serde_json rejects them) rather than failing the whole event;
/// anything without a typed `record_*` is stringified via `record_debug`.
pub(crate) struct EventVisitor {
    pub(crate) message: String,
    pub(crate) fields: Map<String, Value>,
}

impl EventVisitor {
    pub(crate) fn new() -> Self {
        Self {
            message: String::new(),
            fields: Map::new(),
        }
    }

    fn insert(&mut self, name: &str, value: Value) {
        if name == "message" {
            // `message` is conventionally a plain string; coerce anything
            // else so a sink can rely on it being textual. Last write wins.
            self.message = match value {
                Value::String(s) => s,
                other => other.to_string(),
            };
        } else {
            self.fields.insert(name.to_owned(), value);
        }
    }
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field.name(), Value::String(value.to_owned()));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field.name(), Value::Number(value.into()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field.name(), Value::Number(value.into()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field.name(), Value::Bool(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        // serde_json rejects NaN/Inf — drop the field rather than fail the row.
        if let Some(num) = serde_json::Number::from_f64(value) {
            self.insert(field.name(), Value::Number(num));
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.insert(field.name(), Value::String(format!("{value:?}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // EventVisitor::insert holds the only non-trivial logic (message vs.
    // payload split, last-write-wins, non-string `message` coercion). The
    // record_* methods are one-line wrappers and don't earn separate tests.

    #[test]
    fn routes_message_field_to_message() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::String("hello".into()));

        assert_eq!(v.message, "hello");
        assert!(v.fields.is_empty());
    }

    #[test]
    fn routes_non_message_fields_to_payload() {
        let mut v = EventVisitor::new();
        v.insert("status", Value::Number(200.into()));
        v.insert("backend", Value::String("foo".into()));

        assert_eq!(v.message, "");
        assert_eq!(v.fields.len(), 2);
        assert_eq!(v.fields["status"], json!(200));
        assert_eq!(v.fields["backend"], json!("foo"));
    }

    #[test]
    fn message_last_write_wins() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::String("first".into()));
        v.insert("message", Value::String("second".into()));

        assert_eq!(v.message, "second");
    }

    #[test]
    fn coerces_non_string_message_to_string() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::Number(42.into()));

        assert_eq!(v.message, "42");
    }

    #[test]
    fn separates_message_from_other_fields() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::String("the message".into()));
        v.insert("k", Value::Bool(true));

        assert_eq!(v.message, "the message");
        assert_eq!(v.fields.len(), 1);
        // `message` must not also land in `fields` — that would duplicate it.
        assert!(!v.fields.contains_key("message"));
    }

    #[test]
    fn payload_is_none_when_empty() {
        let fields = Map::new();
        let correlation = CorrelationFields::default();
        let ev = StructuredEvent {
            timestamp: SystemTime::UNIX_EPOCH,
            level: Level::INFO,
            message: "hi",
            fields: &fields,
            correlation: &correlation,
        };
        assert!(ev.payload().is_none());
    }

    #[test]
    fn correlation_fields_lookup() {
        let cf = CorrelationFields {
            values: vec![("request_id", "abc".into()), ("trace_id", "xyz".into())],
        };
        assert_eq!(cf.get("request_id"), Some("abc"));
        assert_eq!(cf.get("trace_id"), Some("xyz"));
        assert_eq!(cf.get("missing"), None);
        assert!(!cf.is_empty());
    }
}
