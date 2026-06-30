use crate::event::StructuredEvent;

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: &StructuredEvent<'_>);
}
