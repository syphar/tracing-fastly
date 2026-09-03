mod event;
mod layer;

pub mod serialize;

pub use event::{SpanFields, StructuredEvent, StructuredEventSink};
pub use layer::StructuredEventLayer;
