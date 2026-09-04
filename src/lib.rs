#![warn(missing_docs)]
//! Structured logging for Fastly Compute applications using [`tracing`].
//!
//! [`StructuredEventLayer`] collects fields from tracing events and their
//! active span hierarchy, then passes each self-contained [`StructuredEvent`]
//! to a [`StructuredEventSink`]. Use the included [`providers::datadog`]
//! provider or implement a sink for another provider-specific wire format.

mod event;
mod layer;
/// Ready-to-use sinks for supported logging providers.
pub mod providers;
/// Helpers for serializing and writing structured log records.
pub mod serialize;
#[cfg(any(test, feature = "testing"))]
/// Utilities for testing custom providers.
pub mod testing;

pub use event::{StructuredEvent, StructuredEventSink};
pub use layer::StructuredEventLayer;
