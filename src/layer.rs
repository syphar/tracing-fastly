use crate::event::{JsonFieldVisitor, StructuredEvent, StructuredEventSink, take_message};
use serde_json::{Map, Value};
use std::time::SystemTime;
use tracing::{
    Event, Subscriber,
    span::{Attributes, Id, Record},
};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

/// Produces normalized events with a self-contained set of effective fields.
///
/// Fields from the active span hierarchy are inherited automatically. When the
/// same name occurs more than once, an event field wins over a span field, and
/// an inner span wins over an outer span.
pub struct StructuredEventLayer<K> {
    sink: K,
}

impl<K> StructuredEventLayer<K> {
    pub fn new(sink: K) -> Self {
        Self { sink }
    }
}

#[derive(Default)]
struct SpanState {
    values: Map<String, Value>,
}

impl<S, K> Layer<S> for StructuredEventLayer<K>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    K: StructuredEventSink,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut state = SpanState::default();
        attrs.record(&mut JsonFieldVisitor::new(&mut state.values));
        span.extensions_mut().insert(state);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut ext = span.extensions_mut();
        if let Some(state) = ext.get_mut::<SpanState>() {
            values.record(&mut JsonFieldVisitor::new(&mut state.values));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Map::new();
        event.record(&mut JsonFieldVisitor::new(&mut fields));
        let message = take_message(&mut fields);

        if let Some(scope) = ctx.event_scope(event) {
            // `Scope` iterates from the leaf towards the root. Since event fields
            // are already present and occupied entries are preserved, this gives
            // precedence to the event, then inner spans, then outer spans.
            for span in scope {
                if let Some(state) = span.extensions().get::<SpanState>() {
                    for (name, value) in &state.values {
                        fields.entry(name.clone()).or_insert_with(|| value.clone());
                    }
                }
            }
        }

        let structured = StructuredEvent {
            timestamp: SystemTime::now(),
            level: *event.metadata().level(),
            message: &message,
            fields: &fields,
        };
        self.sink.emit(&structured);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::StructuredEvent;
    use std::sync::{Arc, Mutex};
    use tracing::{info_span, subscriber::with_default};
    use tracing_subscriber::{prelude::*, registry};

    #[derive(Default, Clone)]
    struct Captured {
        message: String,
        fields: Map<String, Value>,
    }

    #[derive(Clone, Default)]
    struct CaptureSink(Arc<Mutex<Option<Captured>>>);

    impl StructuredEventSink for CaptureSink {
        fn emit(&self, event: &StructuredEvent<'_>) {
            *self.0.lock().unwrap() = Some(Captured {
                message: event.message.to_owned(),
                fields: event.fields.clone(),
            });
        }
    }

    fn capture<F: FnOnce()>(f: F) -> Captured {
        let sink = CaptureSink::default();
        let slot = sink.0.clone();
        let layer = StructuredEventLayer::new(sink);
        with_default(registry().with(layer), f);
        slot.lock().unwrap().take().unwrap_or_default()
    }

    #[test]
    fn captures_all_typed_span_fields() {
        let c = capture(|| {
            let span = info_span!(
                "req",
                request_id = "abc-123",
                attempt = 2_u64,
                sampled = true
            );
            let _guard = span.enter();
            tracing::info!("hello");
        });
        assert_eq!(c.fields["request_id"], serde_json::json!("abc-123"));
        assert_eq!(c.fields["attempt"], serde_json::json!(2));
        assert_eq!(c.fields["sampled"], serde_json::json!(true));
        assert_eq!(c.message, "hello");
    }

    #[test]
    fn captures_fields_recorded_after_span_creation() {
        let c = capture(|| {
            let span = info_span!("req", request_id = tracing::field::Empty);
            span.record("request_id", "late-id");
            let _guard = span.enter();
            tracing::info!("hello");
        });
        assert_eq!(c.fields["request_id"], serde_json::json!("late-id"));
    }

    #[test]
    fn innermost_span_wins_for_duplicate_names() {
        let c = capture(|| {
            let outer = info_span!("req", request_id = "root-id");
            let _outer_guard = outer.enter();
            let inner = info_span!("inner", request_id = "inner-id", operation = "lookup");
            let _inner_guard = inner.enter();
            tracing::info!("hello");
        });
        assert_eq!(c.fields["request_id"], serde_json::json!("inner-id"));
        assert_eq!(c.fields["operation"], serde_json::json!("lookup"));
    }

    #[test]
    fn event_fields_override_span_fields() {
        let c = capture(|| {
            let span = info_span!("request", backend = "default");
            let _guard = span.enter();
            tracing::info!(backend = "origin", "hello");
        });

        assert_eq!(c.fields["backend"], serde_json::json!("origin"));
    }
}
