//! Structured `tracing` logging for Fastly Compute (Rust).
//!
//! This crate bridges [`tracing`] events to a Fastly **named log endpoint**,
//! with two pieces that are useful independently:
//!
//! - [`CorrelationLayer`] — a [`tracing_subscriber::Layer`] that turns each
//!   event into a [`StructuredEvent`] (a message + a JSON map of fields) and
//!   propagates chosen span fields (e.g. `request_id`) down to every event
//!   fired within that span's scope. It knows nothing about *where* the event
//!   goes — that's the [`EventSink`] it's constructed with.
//! - [`bq`] — helpers for the common destination: a BigQuery table fed by a
//!   Fastly `logging_bigquery` endpoint. Fire-and-forget NDJSON writing plus
//!   the `serialize_with` functions that encode BigQuery's NDJSON coercion
//!   rules (epoch-seconds timestamps, `JSON`-as-string, …).
//!
//! The **row schema is yours**: it is coupled to your BigQuery table (and, at
//! Thermondo, to the Terraform that declares it), so this crate does not own
//! a concrete row type. You define a `#[derive(Serialize)]` struct whose
//! fields are your columns, reuse the [`bq`] serializers, and write a small
//! [`EventSink`] that maps a [`StructuredEvent`] onto it. See
//! `examples/bigquery.rs` for the full worked example, including an
//! access-log row that reuses the same helpers.
//!
//! # Wiring (sketch)
//!
//! ```ignore
//! use tracing_subscriber::prelude::*;
//!
//! let sink = MyBqSink { service_name: "my_service".into() };
//! let layer = tracing_fastly::CorrelationLayer::new(sink).correlate("request_id");
//!
//! tracing_subscriber::registry()
//!     .with(tracing_subscriber::fmt::layer().compact()) // human stdout for `fastly log-tail`
//!     .with(layer)                                       // structured rows to BigQuery
//!     .init();
//! ```

mod event;
mod layer;
mod sink;

pub mod bq;

pub use event::{CorrelationFields, StructuredEvent};
pub use layer::CorrelationLayer;
pub use sink::EventSink;
