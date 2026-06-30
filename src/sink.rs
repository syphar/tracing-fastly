//! Where structured events go.

use crate::event::StructuredEvent;

/// Receives one [`StructuredEvent`] per `tracing` event from
/// [`CorrelationLayer`](crate::CorrelationLayer).
///
/// This is the extension point that keeps the layer destination-agnostic:
/// implement it to map an event onto your own row type (e.g. a BigQuery
/// schema — see `examples/bigquery.rs`) and write it wherever you like.
///
/// A sink lives inside the global subscriber for the life of the program, so
/// it must be `Send + Sync + 'static`. `emit` is called on the hot path of
/// every event and must not panic or block: a logging failure should be
/// swallowed, never allowed to break the request being served.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &StructuredEvent<'_>);
}
