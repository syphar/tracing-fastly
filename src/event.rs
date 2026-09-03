use serde_json::{Map, Value};
use std::time::SystemTime;
use tracing::{
    Level,
    field::{Field, Visit},
};

/// Structured Event
///
/// Will be populated with the collected info from tracing spans, records and events,
/// then handed to the `StructuredEventSink`.
pub struct StructuredEvent<'a> {
    pub timestamp: SystemTime,
    pub level: Level,
    pub message: &'a str,
    /// The event's effective fields.
    ///
    /// This combines fields recorded directly on the event with fields inherited
    /// from its active span hierarchy. When names collide, event fields take
    /// precedence, followed by the innermost span and then its ancestors.
    pub fields: &'a Map<String, Value>,
}

/// Receives normalized tracing events for conversion into an application-defined format.
pub trait StructuredEventSink: Send + Sync + 'static {
    fn emit(&self, event: &StructuredEvent<'_>);
}

pub(crate) struct JsonFieldVisitor<'a> {
    fields: &'a mut Map<String, Value>,
}

impl<'a> JsonFieldVisitor<'a> {
    pub(crate) fn new(fields: &'a mut Map<String, Value>) -> Self {
        Self { fields }
    }

    fn insert(&mut self, name: &str, value: Value) {
        self.fields.insert(name.to_owned(), value);
    }
}

impl Visit for JsonFieldVisitor<'_> {
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
        if let Some(num) = serde_json::Number::from_f64(value) {
            self.insert(field.name(), Value::Number(num));
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.insert(field.name(), Value::String(format!("{value:?}")));
    }
}

pub(crate) fn take_message(fields: &mut Map<String, Value>) -> String {
    fields
        .remove("message")
        .map(|value| match value {
            Value::String(message) => message,
            other => other.to_string(),
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn routes_message_field_to_message() {
        let mut fields = Map::new();
        fields.insert("message".into(), Value::String("hello".into()));

        assert_eq!(take_message(&mut fields), "hello");
        assert!(fields.is_empty());
    }

    #[test]
    fn routes_non_message_fields_to_payload() {
        let mut fields = Map::new();
        let mut visitor = JsonFieldVisitor::new(&mut fields);
        visitor.insert("status", Value::Number(200.into()));
        visitor.insert("backend", Value::String("foo".into()));

        assert_eq!(fields.len(), 2);
        assert_eq!(fields["status"], json!(200));
        assert_eq!(fields["backend"], json!("foo"));
    }

    #[test]
    fn coerces_non_string_message_to_string() {
        let mut fields = Map::from_iter([("message".into(), Value::Number(42.into()))]);

        assert_eq!(take_message(&mut fields), "42");
    }

    #[test]
    fn separates_message_from_other_fields() {
        let mut fields = Map::from_iter([
            ("message".into(), Value::String("the message".into())),
            ("k".into(), Value::Bool(true)),
        ]);

        assert_eq!(take_message(&mut fields), "the message");
        assert_eq!(fields.len(), 1);

        assert!(!fields.contains_key("message"));
    }
}
