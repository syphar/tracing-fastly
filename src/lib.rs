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
//! # Easiest setup: compact lines to stdout
//!
//! When you just want logs off the edge with no BigQuery schema to maintain, a
//! compact stdout `fmt` layer is enough (on Compute, the platform forwards
//! stdout to your configured logging endpoint):
//!
//! ```ignore
//! use tracing_subscriber::{EnvFilter, fmt, prelude::*};
//!
//! tracing_subscriber::registry()
//!     .with(EnvFilter::from_default_env())
//!     .with(fmt::layer().compact().with_ansi(false))
//!     .init();
//! ```
//!
//! To write straight to a named Fastly endpoint instead — or tee to both
//! stdout and an endpoint from one layer — point the writer at an
//! [`EndpointWriter`]. The destination is just the writer; the format stays in
//! one `fmt` layer. See the [`setup`] module for those variants.
//!
//! # Full setup: structured rows to BigQuery
//!
//! ```ignore
//! use tracing_subscriber::{fmt, prelude::*};
//!
//! let sink = MyBqSink { service_name: "my_service".into() };
//! let layer = tracing_fastly::CorrelationLayer::new(sink).correlate("request_id");
//!
//! tracing_subscriber::registry()
//!     .with(fmt::layer().compact().with_ansi(false)) // human stdout for `fastly log-tail`
//!     .with(layer)                                    // structured rows to BigQuery
//!     .init();
//! ```

mod event;
mod layer;
mod sink;

pub mod bq;
pub mod setup;

pub use event::{CorrelationFields, StructuredEvent};
pub use layer::CorrelationLayer;
pub use setup::EndpointWriter;
pub use sink::EventSink;
