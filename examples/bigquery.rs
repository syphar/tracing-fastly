//! Worked example: ship `tracing` events to a BigQuery `trace_logs` table.
//!
//! This is the half of the original module that is **yours to own** — the row
//! schema. It's coupled to a specific BigQuery table (at Thermondo, to the
//! Terraform in `modules/fastly/bq_log_sink/` that declares the table, schema,
//! and endpoint name), so the library deliberately doesn't define it. It gives
//! you the serde coercion helpers ([`tracing_fastly::bq`]) and the
//! [`EventSink`] seam; you bring the columns.
//!
//! [`BqTraceSink`] maps each [`StructuredEvent`] the [`CorrelationLayer`]
//! produces onto a [`TraceLog`] row and ships it. For a per-request access-log
//! row that reuses the same `bq` helpers, see `examples/access_log.rs`.
//!
//! Endpoint names must match the `logging_bigquery` blocks on the Fastly
//! service. Writing to an unconfigured endpoint is dropped at the edge.
//!
//! Run `cargo build --example bigquery` to type-check; the Fastly hostcalls
//! only do anything inside the Compute runtime.

use serde::Serialize;
use serde_with::{DisplayFromStr, serde_as, skip_serializing_none};
use std::time::SystemTime;
use tracing_fastly::{CorrelationLayer, EventSink, StructuredEvent, bq};
use tracing_subscriber::prelude::*;

const TRACE_LOG_ENDPOINT: &str = "bq_trace_logs";

/// Install the global subscriber: a compact stdout layer for `fastly
/// log-tail`, plus the BigQuery trace sink (only when its endpoint exists).
fn setup_logging(service_name: &str) {
    // `Option<Layer>` is itself a `Layer` and no-ops when `None`, so without
    // the endpoint we skip the per-event serialization cost entirely.
    let bq_layer = bq::endpoint_configured(TRACE_LOG_ENDPOINT).then(|| {
        CorrelationLayer::new(BqTraceSink {
            service_name: service_name.to_owned(),
            endpoint: TRACE_LOG_ENDPOINT,
        })
        .correlate("request_id")
    });

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact().with_ansi(false))
        .with(bq_layer)
        .init();
}

/// Maps each [`StructuredEvent`] onto a [`TraceLog`] row and ships it. The
/// destination endpoint lives here, on the sink — the `EventSink` trait itself
/// stays endpoint-agnostic.
struct BqTraceSink {
    service_name: String,
    endpoint: &'static str,
}

impl EventSink for BqTraceSink {
    fn emit(&self, event: &StructuredEvent<'_>) {
        // The library hands us the non-message fields as a map; wrap them as a
        // JSON object for the `payload` column (omitted when empty).
        let payload = event
            .payload()
            .map(|fields| serde_json::Value::Object(fields.clone()));

        let row = TraceLog {
            timestamp: event.timestamp,
            service_name: &self.service_name,
            request_id: event.correlation.get("request_id"),
            level: event.level,
            message: event.message,
            payload: payload.as_ref(),
        };
        bq::write_ndjson_row(self.endpoint, &row);
    }
}

/// One row in the `trace_logs` BigQuery table. The serialized JSON *is* the
/// row: keys are columns, and they must match the BQ schema exactly.
#[skip_serializing_none]
#[serde_as]
#[derive(Serialize)]
struct TraceLog<'a> {
    #[serde(serialize_with = "bq::ser_unix_seconds")]
    timestamp: SystemTime,
    service_name: &'a str,
    request_id: Option<&'a str>,
    #[serde_as(as = "DisplayFromStr")]
    level: tracing::Level,
    message: &'a str,
    // BQ `JSON` columns ingested via NDJSON must be a JSON-encoded *string*,
    // not a nested object — see [`bq::ser_json_as_string`].
    #[serde(serialize_with = "bq::ser_json_as_string")]
    payload: Option<&'a serde_json::Value>,
}

fn main() {
    setup_logging("example_service");

    // A handler opens a request span, fills in the id, and logs within it; the
    // events inherit `request_id` without threading it through each macro.
    let span = tracing::info_span!("request", request_id = tracing::field::Empty);
    span.record("request_id", "req-abc-123");
    let _guard = span.enter();

    tracing::info!(status = 200, backend = "origin", "handled request");
    tracing::warn!(reason = "stale", "cache miss");
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::time::Duration;

    // Schema-shape tests live with the schema. Run via `cargo test --example
    // bigquery`. The key invariant: the serialized key set must match the BQ
    // table's columns exactly, or rows are dropped on load.

    #[test]
    fn trace_log_columns_and_payload_is_a_string() {
        let payload = json!({ "k": 1 });
        let row = TraceLog {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_millis(1500),
            service_name: "svc",
            request_id: Some("rid"),
            level: tracing::Level::INFO,
            message: "hi",
            payload: Some(&payload),
        };

        let v = serde_json::to_value(&row).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["level", "message", "payload", "request_id", "service_name", "timestamp"]
        );

        let s = v["payload"].as_str().expect("payload must be a JSON string");
        assert_eq!(serde_json::from_str::<serde_json::Value>(s).unwrap(), json!({ "k": 1 }));
    }

    #[test]
    fn trace_log_omits_none_payload_and_request_id() {
        let row = TraceLog {
            timestamp: SystemTime::UNIX_EPOCH,
            service_name: "svc",
            request_id: None,
            level: tracing::Level::INFO,
            message: "",
            payload: None,
        };
        let v = serde_json::to_value(&row).unwrap();
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("request_id"));
        assert!(!obj.contains_key("payload"));
    }
}
