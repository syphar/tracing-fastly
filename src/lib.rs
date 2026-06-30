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
//! `examples/bigquery.rs` for the full worked example, and
//! `examples/access_log.rs` for a per-request access-log row that reuses the
//! same helpers.
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
//! stdout and an endpoint from one layer — set the layer's *writer*. `fmt`
//! takes a [`MakeWriter`](tracing_subscriber::fmt::MakeWriter) (a per-line
//! writer factory), and any closure `Fn() -> impl Write` is one;
//! `fastly::log::Endpoint` is a `Write`, so:
//!
//! ```ignore
//! use fastly::log::Endpoint;
//! use tracing_subscriber::{fmt, fmt::writer::MakeWriterExt, prelude::*};
//!
//! // Straight to the endpoint:
//! let to_endpoint = fmt::layer().compact().with_ansi(false)
//!     .with_writer(|| Endpoint::from_name("my_logs"));
//!
//! // Or teed to stdout *and* the endpoint — one format, two destinations:
//! let writer = std::io::stdout.and(|| Endpoint::from_name("my_logs"));
//! let teed = fmt::layer().compact().with_ansi(false).with_writer(writer);
//! ```
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

pub use event::{CorrelationFields, StructuredEvent};
pub use layer::CorrelationLayer;
pub use sink::EventSink;
