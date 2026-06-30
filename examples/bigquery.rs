//! Worked example: ship `tracing` events and access logs to BigQuery.
//!
//! This is the half of the original module that is **yours to own** — the row
//! schemas. They're coupled to specific BigQuery tables (at Thermondo, to the
//! Terraform in `modules/fastly/bq_log_sink/` that declares the tables,
//! schemas, and endpoint names), so the library deliberately doesn't define
//! them. It gives you the serde coercion helpers ([`tracing_fastly::bq`]) and
//! the [`EventSink`] seam; you bring the columns.
//!
//! Two rows are shown:
//! - [`TraceLog`] — one row per `tracing` event, produced by [`BqTraceSink`].
//! - [`AccessLog`] — one row per request, emitted at request end by
//!   [`emit_access_log`], reusing the same `bq` helpers. Access logging isn't
//!   a `tracing` concern, so it's a free-standing recipe rather than library
//!   API.
//!
//! Endpoint names must match the `logging_bigquery` blocks on the Fastly
//! service. Writing to an unconfigured endpoint is dropped at the edge, so
//! emitting unconditionally is safe.
//!
//! Run `cargo build --example bigquery` to type-check; the Fastly hostcalls
//! only do anything inside the Compute runtime.

use fastly::{
    Request, Response,
    http::{
        self, HeaderName, StatusCode, Url,
        header::{REFERER, USER_AGENT},
    },
};
use serde::Serialize;
use serde_with::{DisplayFromStr, serde_as, skip_serializing_none};
use std::{
    net::{self, IpAddr},
    time::{Duration, Instant, SystemTime},
};
use tracing::warn;
use tracing_fastly::{CorrelationLayer, EventSink, StructuredEvent, bq};
use tracing_subscriber::prelude::*;

const ACCESS_LOG_ENDPOINT: &str = "bq_access_logs";
const TRACE_LOG_ENDPOINT: &str = "bq_trace_logs";

const FASTLY_CLIENT_IP: HeaderName = HeaderName::from_static("fastly-client-ip");
const X_CACHE: HeaderName = HeaderName::from_static("x-cache");

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Trace log: one row per tracing event
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Access log: one row per request (a recipe, not library API)
// ---------------------------------------------------------------------------

/// One row in the `access_logs` BigQuery table.
#[skip_serializing_none]
#[serde_as]
#[derive(Serialize)]
struct AccessLog<'a> {
    #[serde(serialize_with = "bq::ser_unix_seconds")]
    timestamp: SystemTime,
    service_name: &'a str,
    request_id: Option<&'a str>,
    host: Option<&'a str>,
    #[serde_as(as = "DisplayFromStr")]
    method: &'a http::Method,
    #[serde_as(as = "DisplayFromStr")]
    url: &'a Url,
    #[serde(serialize_with = "bq::ser_http_status")]
    status: StatusCode,
    #[serde(serialize_with = "bq::ser_duration_ms", rename = "response_time_ms")]
    response_time: Duration,
    bytes_written: Option<usize>,
    cache_status: Option<&'a str>,
    client_ip: Option<&'a IpAddr>,
    client_country: Option<&'a str>,
    user_agent: Option<&'a str>,
    referer: Option<&'a str>,
    tls_protocol: Option<&'a str>,
    backend_name: Option<&'a str>,
    fastly_pop: Option<&'a str>,
    fastly_server: Option<&'a str>,
}

/// Request-side fields snapshotted at the start of a request, since the
/// `Request` is consumed downstream.
struct RequestCapture {
    started_at: Instant,
    method: http::Method,
    url: Url,
    client_ip: Option<IpAddr>,
    user_agent: Option<String>,
    referer: Option<String>,
    tls_protocol: Option<String>,
}

impl RequestCapture {
    fn host(&self) -> Option<&str> {
        self.url.host_str()
    }

    fn client_country(&self) -> Option<String> {
        self.client_ip
            .and_then(fastly::geo::geo_lookup)
            .map(|geo| geo.country_code().to_owned())
    }
}

impl From<&Request> for RequestCapture {
    fn from(req: &Request) -> Self {
        let client_ip = req
            .get_header_str_lossy(FASTLY_CLIENT_IP)
            .and_then(|s| match s.parse::<net::IpAddr>() {
                Ok(ip) => Some(ip),
                Err(err) => {
                    warn!(?err, value = s.as_ref(), "error parsing fastly-client-ip");
                    None
                }
            });

        Self {
            started_at: Instant::now(),
            method: req.get_method().to_owned(),
            url: req.get_url().to_owned(),
            client_ip,
            user_agent: req.get_header_str_lossy(USER_AGENT).map(|s| s.to_string()),
            referer: req.get_header_str_lossy(REFERER).map(|s| s.to_string()),
            tls_protocol: req.get_tls_protocol().ok().flatten().map(|s| s.to_owned()),
        }
    }
}

/// Emit one access-log row at request end. Logging failures are swallowed.
fn emit_access_log(
    capture: &RequestCapture,
    request_id: &str,
    response: &Response,
    backend_name: Option<&str>,
    service_name: &str,
) {
    let response_time = capture.started_at.elapsed();

    // One-line human summary for `fastly log-tail`. Plain `println!` so it
    // doesn't also round-trip through the trace sink.
    println!(
        "access status={} method={} url={} response_time_ms={} backend={} request_id={request_id}",
        response.get_status().as_u16(),
        capture.method,
        capture.url,
        response_time.as_millis(),
        backend_name.unwrap_or("-"),
    );

    if !bq::endpoint_configured(ACCESS_LOG_ENDPOINT) {
        return;
    }

    let client_country = capture.client_country();
    let cache_status = response.get_header_str_lossy(X_CACHE);

    let row = AccessLog {
        timestamp: SystemTime::now(),
        service_name,
        request_id: Some(request_id),
        host: capture.host(),
        method: &capture.method,
        url: &capture.url,
        status: response.get_status(),
        response_time,
        bytes_written: response.get_content_length(),
        cache_status: cache_status.as_deref(),
        client_ip: capture.client_ip.as_ref(),
        client_country: client_country.as_deref(),
        user_agent: capture.user_agent.as_deref(),
        referer: capture.referer.as_deref(),
        tls_protocol: capture.tls_protocol.as_deref(),
        backend_name,
        fastly_pop: non_empty(fastly::compute_runtime::pop()),
        fastly_server: non_empty(fastly::compute_runtime::hostname()),
    };
    bq::write_ndjson_row(ACCESS_LOG_ENDPOINT, &row);
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

// ---------------------------------------------------------------------------

fn main() {
    setup_logging("example_service");

    // A handler opens a request span, fills in the id, and logs within it;
    // the event inherits `request_id` without threading it through the macro.
    let span = tracing::info_span!("request", request_id = tracing::field::Empty);
    span.record("request_id", "req-abc-123");
    let _guard = span.enter();

    tracing::info!(status = 200, backend = "origin", "handled request");

    let req = Request::new(http::Method::GET, "https://example.thermondo.de/");
    let capture = RequestCapture::from(&req);
    let response = Response::from_status(StatusCode::OK);
    emit_access_log(&capture, "req-abc-123", &response, Some("origin"), "example_service");
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::net::Ipv4Addr;

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

    #[test]
    fn access_log_column_set_matches_schema() {
        let row = AccessLog {
            timestamp: SystemTime::now(),
            service_name: "svc",
            request_id: Some("rid-1"),
            host: Some("example.thermondo.de"),
            method: &http::Method::GET,
            url: &Url::parse("https://example.thermondo.de/").unwrap(),
            status: StatusCode::OK,
            response_time: Duration::from_millis(12),
            bytes_written: Some(1024),
            cache_status: Some("PASS"),
            client_ip: Some(&IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))),
            client_country: Some("DE"),
            user_agent: Some("curl/8"),
            referer: Some("https://ref"),
            tls_protocol: Some("TLSv1.3"),
            backend_name: Some("my-backend"),
            fastly_pop: Some("FRA"),
            fastly_server: Some("cache-fra19151"),
        };

        let v = serde_json::to_value(&row).unwrap();
        assert!(v["timestamp"].is_number());
        assert!(v["status"].is_number());
        assert!(v["response_time_ms"].is_number());

        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "backend_name",
                "bytes_written",
                "cache_status",
                "client_country",
                "client_ip",
                "fastly_pop",
                "fastly_server",
                "host",
                "method",
                "referer",
                "request_id",
                "response_time_ms",
                "service_name",
                "status",
                "timestamp",
                "tls_protocol",
                "url",
                "user_agent",
            ]
        );
    }
}
