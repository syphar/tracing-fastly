mod event;
mod layer;

pub mod serialize;

pub use event::{CorrelationFields, StructuredEvent, StructuredEventSink};
pub use layer::CorrelationLayer;
