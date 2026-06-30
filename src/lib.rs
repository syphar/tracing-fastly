mod event;
mod layer;
mod sink;

pub mod bq;

pub use event::{CorrelationFields, StructuredEvent};
pub use layer::CorrelationLayer;
pub use sink::EventSink;
