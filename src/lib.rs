mod event;
mod layer;

pub mod serialize;

pub use event::{CorrelationFields, EventSink, StructuredEvent};
pub use layer::CorrelationLayer;
