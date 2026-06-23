//! Tracing setup.
//!
//! Concepts from [`tracing`] used here:
//! - **event** — a single point-in-time log record (`info!`, `warn!`, …),
//!   carrying a `message` plus arbitrary key/value fields.
//!   ([`tracing::Event`])
//! - **span** — a unit of work events happen *within*; spans nest, forming a
//!   scope from root to leaf. Our `request_span` wraps one request.
//!   ([`tracing::Span`])
//! - **field / record** — spans carry fields too. A field declared
//!   `field::Empty` at creation can be filled later with `span.record(...)`.
//!   ([`tracing::field`])
//! - **subscriber** — the global sink all events/spans flow to.
//!   ([`tracing::Subscriber`])
//! - **layer** — a composable slice of subscriber behaviour; layers stack and
//!   each sees every event. A [`Visit`] implementation extracts field values.
//!   ([`tracing_subscriber::Layer`])
//!
//! Two sinks see every event: a compact stdout fmt layer for live debugging
//! via `fastly log-tail`, and [`BqTraceLayer`], which turns each event into a
//! `bq_trace_logs` BigQuery row (attached only when that Fastly endpoint
//! exists).
//!
//! The non-obvious part is request correlation. `main.rs` opens a
//! `request_span` and records `request_id` on it; every event fired under that
//! span must carry that id without callers threading it through each macro.
//! [`BqTraceLayer`] handles this by stashing a [`SpanState`] in each span's
//! `extensions()` and, per event, walking the span scope to the nearest
//! ancestor that has a `request_id`.

use crate::bq_logging;
use serde_json::{Map, Value};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
    level_filters::LevelFilter,
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{
    EnvFilter, fmt,
    layer::{Context, Layer},
    prelude::*,
    registry::LookupSpan,
};

pub(crate) const TRACE_REQUEST_ID: &str = "request_id";

/// Install the global tracing subscriber: a compact stdout layer plus
/// [`BqTraceLayer`] (only when the `bq_trace_logs` endpoint is configured).
pub(crate) fn setup_logging() {
    // `Option<L: Layer>` is itself a `Layer` and no-ops when `None`, so
    // without a sink the per-event visitor/serialization cost is skipped.
    let bq_trace_layer =
        bq_logging::trace_endpoint_configured().then(|| BqTraceLayer::new("DUMMY"));

    let subscriber = tracing_subscriber::registry()
        .with(fmt::layer().compact())
        .with(bq_trace_layer);
    // .with(
    //     EnvFilter::builder()
    //         .with_default_directive(LevelFilter::INFO.into())
    //         .parse_lossy(&config.log_directives),
    // );

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set subscriber");
}

/// `Layer` that turns each event into a `bq_trace_logs` row.
///
/// - `on_new_span` allocates a [`SpanState`] and captures `request_id` from
///   the span's construction-time fields.
/// - `on_record` captures `request_id` filled in later via `span.record(...)`
///   — `main.rs` creates `request_span` with `request_id = Empty` and sets it
///   afterwards.
/// - `on_event` resolves `request_id` from the nearest ancestor span, splits
///   the event into message + fields via [`EventVisitor`], and hands them to
///   [`bq_logging::emit_trace_event`].
///
/// `service_name` is owned because the layer is `'static` once installed.
struct BqTraceLayer {
    service_name: String,
}

impl BqTraceLayer {
    fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
        }
    }
}

/// Per-span state in `span.extensions()` so an event can inherit the
/// `request_id` from an ancestor span. Written by [`RequestIdVisitor`] (from
/// `on_new_span` and/or `on_record`), read from `on_event`.
#[derive(Default)]
struct SpanState {
    request_id: Option<String>,
}

/// Pulls `request_id` out of a span's fields into `out`. Used from both
/// `on_new_span` (`Attributes`) and `on_record` (`Record`), which expose
/// fields through the same [`Visit`] trait.
///
/// `request_id` normally arrives as `&str` (`record_str`). `record_debug` is
/// the fallback for other types; a `&str`'s Debug repr is quoted, so we strip
/// the surrounding quotes.
struct RequestIdVisitor<'a> {
    out: &'a mut Option<String>,
}

impl Visit for RequestIdVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == TRACE_REQUEST_ID {
            *self.out = Some(value.to_owned());
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == TRACE_REQUEST_ID {
            let s = format!("{value:?}");
            *self.out = Some(s.trim_matches('"').to_owned());
        }
    }
}

/// Splits one event's fields into the two columns the BQ row needs:
///
/// - `message` — the STRING `message` column, from either tracing's
///   synthesized format-string `message` field (`info!("hello {x}")`) or an
///   explicit `message = "..."` field (last write wins).
/// - `fields` — everything else, as the JSON `payload` column. Must never
///   include `message`, or it would be duplicated into the payload.
///
/// The `record_*` methods wrap [`Self::insert`], which holds the
/// message/payload split and the STRING coercion of non-string messages.
/// `f64` NaN/Inf are dropped (serde_json rejects them) rather than failing the
/// row; anything not covered by a typed `record_*` is stringified via
/// `record_debug`.
struct EventVisitor {
    message: String,
    fields: Map<String, Value>,
}

impl EventVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: Map::new(),
        }
    }

    fn insert(&mut self, name: &str, value: Value) {
        if name == "message" {
            // The `message` column is STRING: keep strings as-is, stringify
            // anything else. Last write wins.
            if let Value::String(s) = value {
                self.message = s;
            } else {
                self.message = value.to_string();
            }
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

impl<S> Layer<S> for BqTraceLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut state = SpanState::default();
        attrs.record(&mut RequestIdVisitor {
            out: &mut state.request_id,
        });
        span.extensions_mut().insert(state);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        // `request_id` is set via `span.record(...)` after creation, so catch
        // it here too, not only in `on_new_span`.
        let Some(span) = ctx.span(id) else { return };
        let mut ext = span.extensions_mut();
        if let Some(state) = ext.get_mut::<SpanState>() {
            values.record(&mut RequestIdVisitor {
                out: &mut state.request_id,
            });
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = EventVisitor::new();
        event.record(&mut visitor);

        let request_id = ctx.event_scope(event).and_then(|scope| {
            scope.from_root().find_map(|span| {
                span.extensions()
                    .get::<SpanState>()
                    .and_then(|s| s.request_id.clone())
            })
        });

        bq_logging::emit_trace_event(
            &self.service_name,
            request_id.as_deref(),
            *event.metadata().level(),
            &visitor.message,
            if visitor.fields.is_empty() {
                None
            } else {
                Some(Value::Object(visitor.fields))
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tracing::{info_span, subscriber::with_default};
    use tracing_subscriber::{layer::SubscriberExt, registry};

    // EventVisitor::insert encapsulates the only non-trivial logic in
    // the visitor (message vs. payload split, last-write-wins,
    // non-string `message` coercion). The record_* methods are
    // one-line wrappers around `insert` plus a value-type conversion
    // and don't earn separate tests.

    // Drive RequestIdVisitor through a real tracing pipeline — a
    // standalone `Field` isn't constructible outside tracing's
    // callsite machinery, so exercise the visitor via the layer it
    // belongs to.
    fn capture_request_id_for_event<F: FnOnce()>(f: F) -> Option<String> {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Capture(Arc<Mutex<Option<Option<String>>>>);
        impl<S> Layer<S> for Capture
        where
            S: Subscriber + for<'lookup> LookupSpan<'lookup>,
        {
            fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
                let rid = ctx.event_scope(event).and_then(|scope| {
                    scope.from_root().find_map(|span| {
                        span.extensions()
                            .get::<SpanState>()
                            .and_then(|s| s.request_id.clone())
                    })
                });
                *self.0.lock().unwrap() = Some(rid);
            }
        }

        let slot: Arc<Mutex<Option<Option<String>>>> = Arc::default();
        let capture = Capture(slot.clone());
        let subscriber = registry().with(BqTraceLayer::new("test")).with(capture);
        with_default(subscriber, f);
        slot.lock().unwrap().take().flatten()
    }

    #[test]
    fn request_id_captured_from_span_creation() {
        let rid = capture_request_id_for_event(|| {
            let span = info_span!("req", request_id = "abc-123");
            let _g = span.enter();
            tracing::info!("hello");
        });
        assert_eq!(rid.as_deref(), Some("abc-123"));
    }

    #[test]
    fn request_id_captured_when_recorded_after_span_creation() {
        let rid = capture_request_id_for_event(|| {
            let span = info_span!("req", request_id = tracing::field::Empty);
            span.record("request_id", "late-id");
            let _g = span.enter();
            tracing::info!("hello");
        });
        assert_eq!(rid.as_deref(), Some("late-id"));
    }

    #[test]
    fn request_id_inherited_from_ancestor_span() {
        let rid = capture_request_id_for_event(|| {
            let outer = info_span!("req", request_id = "root-id");
            let _g = outer.enter();
            let inner = info_span!("inner");
            let _g2 = inner.enter();
            tracing::info!("hello");
        });
        assert_eq!(rid.as_deref(), Some("root-id"));
    }

    #[test]
    fn request_id_absent_when_no_ancestor_has_it() {
        let rid = capture_request_id_for_event(|| {
            let span = info_span!("plain");
            let _g = span.enter();
            tracing::info!("hello");
        });
        assert_eq!(rid, None);
    }

    #[test]
    fn request_id_ignores_other_fields() {
        let rid = capture_request_id_for_event(|| {
            let span = info_span!("req", other = "nope", request_id = "real");
            let _g = span.enter();
            tracing::info!("hello");
        });
        assert_eq!(rid.as_deref(), Some("real"));
    }

    #[test]
    fn event_visitor_routes_message_field_to_message() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::String("hello".into()));

        assert_eq!(v.message, "hello");
        assert!(v.fields.is_empty());
    }

    #[test]
    fn event_visitor_routes_non_message_fields_to_payload() {
        let mut v = EventVisitor::new();
        v.insert("status", Value::Number(200.into()));
        v.insert("backend", Value::String("foo".into()));

        assert_eq!(v.message, "");
        assert_eq!(v.fields.len(), 2);
        assert_eq!(v.fields["status"], json!(200));
        assert_eq!(v.fields["backend"], json!("foo"));
    }

    #[test]
    fn event_visitor_message_last_write_wins() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::String("first".into()));
        v.insert("message", Value::String("second".into()));

        assert_eq!(v.message, "second");
    }

    #[test]
    fn event_visitor_coerces_non_string_message_to_string() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::Number(42.into()));

        // Non-string `message` should still produce a textual message
        // — the BQ column is STRING.
        assert_eq!(v.message, "42");
    }

    #[test]
    fn event_visitor_separates_message_from_other_fields() {
        let mut v = EventVisitor::new();
        v.insert("message", Value::String("the message".into()));
        v.insert("k", Value::Bool(true));

        assert_eq!(v.message, "the message");
        assert_eq!(v.fields.len(), 1);
        assert_eq!(v.fields["k"], json!(true));
        // `message` must not also end up in `fields` — that would
        // duplicate it into the payload JSON column.
        assert!(!v.fields.contains_key("message"));
    }
}
