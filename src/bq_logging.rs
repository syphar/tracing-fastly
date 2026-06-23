//! Per-request BigQuery logging.
//!
//! Owns the row schemas, JSON serialization, and writes to the
//! `bq_access_logs` / `bq_trace_logs` Fastly endpoints. Endpoint names
//! must match the `logging_bigquery` blocks declared by the Fastly
//! service in Terraform (see `modules/fastly/bq_log_sink` and its
//! consumers). Writes to an endpoint not configured on the service are
//! silently dropped at the edge, so it's safe to emit unconditionally.
//!
//! The trace log is fed by a `tracing_subscriber::Layer` that lives in
//! `crate::logging`; it calls [`emit_trace_event`] once per event.

use fastly::http::HeaderName;
use fastly::{
    Request, Response,
    http::{
        self, StatusCode, Url,
        header::{REFERER, USER_AGENT},
    },
    log::Endpoint,
};
use serde::{Serialize, Serializer};
use serde_with::{DisplayFromStr, serde_as, skip_serializing_none};
use std::{
    io::Write,
    net::{self, IpAddr},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tracing::warn;

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const X_ROBOTS_TAG: HeaderName = HeaderName::from_static("x-robots-tag");
const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
// https://www.fastly.com/documentation/guides/concepts/compression/#dynamic-compression
const X_COMPRESS_HINT: HeaderName = HeaderName::from_static("x-compress-hint");
const SURROGATE_CONTROL: HeaderName = HeaderName::from_static("surrogate-control");
const SURROGATE_KEY: HeaderName = HeaderName::from_static("surrogate-key");
const FASTLY_CLIENT_IP: HeaderName = HeaderName::from_static("fastly-client-ip");
const X_CACHE: HeaderName = HeaderName::from_static("x-cache");

/// Hardcoded names of the logging endpoint for access & trace logs.
/// Configured / defined with schema in
/// infra-global,  modules/fastly/bq_log_sink/
/// * bigquery tables & schemas
/// * endpoint names for the fastly service
const ACCESS_LOG_ENDPOINT: &str = "bq_access_logs";
const TRACE_LOG_ENDPOINT: &str = "bq_trace_logs";

pub(crate) fn trace_endpoint_configured() -> bool {
    Endpoint::try_from_name(TRACE_LOG_ENDPOINT).is_ok()
}

pub(crate) fn access_endpoint_configured() -> bool {
    Endpoint::try_from_name(ACCESS_LOG_ENDPOINT).is_ok()
}

/// One row in the `access_logs` BigQuery table.
///
/// Field names and types must stay in lockstep with the BQ table
/// schema, which is the source of truth and lives in the Terraform
/// repo: `modules/fastly/bq_log_sink/schema.json`. Fastly ships rows
/// as newline-delimited JSON and BigQuery ingests them via load jobs;
/// a field here with no matching column there causes the row to be
/// dropped (BQ's NDJSON load defaults to strict schema matching).
///
/// The serialized JSON *is* the BigQuery row: each top-level key maps
/// to a column, and value types must coerce to the column types.
/// `TIMESTAMP` columns accept either RFC 3339 strings or a JSON number
/// of Unix epoch seconds — we use the latter.
#[skip_serializing_none]
#[serde_as]
#[derive(Serialize)]
struct AccessLog<'a> {
    #[serde(serialize_with = "ser_unix_seconds")]
    timestamp: SystemTime,
    service_name: &'a str,
    request_id: Option<&'a str>,
    host: Option<&'a str>,
    #[serde_as(as = "DisplayFromStr")]
    method: &'a http::Method,
    #[serde_as(as = "DisplayFromStr")]
    url: &'a Url,
    #[serde(serialize_with = "ser_http_status")]
    status: StatusCode,
    #[serde(serialize_with = "ser_duration_ms", rename = "response_time_ms")]
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

/// One row in the `trace_logs` BigQuery table.
///
/// Same contract as [`AccessLog`]: field names and types must match
/// the BQ table schema in `modules/fastly/bq_log_sink/schema_trace.json`
/// (Terraform repo), and the serialized JSON is the row Fastly ships
/// to BigQuery via newline-delimited JSON load jobs. `payload` is a
/// free-form `JSON` column — the only place where new structure can
/// be added without a schema change. It must be emitted as a
/// JSON-encoded *string* (see [`ser_json_as_string`]).
#[skip_serializing_none]
#[serde_as]
#[derive(Serialize)]
struct TraceLog<'a> {
    #[serde(serialize_with = "ser_unix_seconds")]
    timestamp: SystemTime,
    service_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<&'a str>,
    #[serde_as(as = "DisplayFromStr")]
    level: tracing::Level,
    message: &'a str,
    // BQ's `JSON` column ingested via NDJSON loads (Fastly's shipping
    // format) requires the value to be a JSON-encoded *string*, not a
    // nested object. Sending a nested object silently drops the row.
    #[serde(serialize_with = "ser_json_as_string")]
    payload: Option<&'a serde_json::Value>,
}

/// serialize http status code as u16
fn ser_http_status<S: Serializer>(status: &http::StatusCode, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u16(status.as_u16())
}

/// serialize a duration as milliseconds
fn ser_duration_ms<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(duration.as_secs_f64() * 1000.0)
}

/// BigQuery's `JSON` column type expects the value as a
/// JSON-encoded string in NDJSON loads. A nested object would
/// type-mismatch and cause BQ to drop the row.
fn ser_json_as_string<S: Serializer>(
    v: &Option<&serde_json::Value>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match v {
        Some(val) => s.serialize_str(&val.to_string()),
        None => s.serialize_none(),
    }
}

/// Unix epoch seconds (JSON number) so BQ TIMESTAMP coerces. A
/// pre-epoch value falls back to 0.0 rather than failing the row.
fn ser_unix_seconds<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    s.serialize_f64(secs)
}

/// Snapshot of request-side fields captured at the start of a request.
/// The original `Request` is consumed downstream so the few fields we
/// need for the access log are copied up front.
pub(crate) struct RequestCapture {
    started_at: Instant,
    method: http::Method,
    url: Url,
    client_ip: Option<IpAddr>,
    user_agent: Option<String>,
    referer: Option<String>,
    tls_protocol: Option<String>,
}

impl RequestCapture {
    pub(crate) fn host(&self) -> Option<&str> {
        self.url.host_str()
    }

    pub(crate) fn client_country(&self) -> Option<String> {
        self.client_ip
            .and_then(fastly::geo::geo_lookup)
            .map(|geo| geo.country_code().to_owned())
    }
}

impl From<&Request> for RequestCapture {
    fn from(req: &Request) -> Self {
        // Read `Fastly-Client-IP` directly; `main` has already
        // normalized it (filling from `get_client_ip_addr()` when the
        // edge header was absent) so this is the single source of
        // truth. Once shielding is enabled this is still the real
        // client IP at the shield POP — the TCP peer would be the
        // edge POP.
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
            // Surfaced as the BQ `tls_protocol` column. None on the
            // local/HTTP test path; populated with e.g. "TLSv1.3" in
            // production.
            tls_protocol: req.get_tls_protocol().ok().flatten().map(|s| s.to_owned()),
        }
    }
}

/// Emit one access-log row. Always called at the end of the request,
/// once the final `Response` is known. Logging failures are swallowed —
/// a broken sink must not break the request itself.
pub(crate) fn emit_access_log(
    capture: &RequestCapture,
    request_id: &str,
    response: &Response,
    backend_name: Option<&str>,
    service_name: &str,
) {
    let response_time_ms = capture.started_at.elapsed();

    // One-line summary to stdout for `fastly log-tail`. Goes alongside
    // the full BQ row — stdout is for humans, BQ has the full schema.
    // Plain `println!` rather than `tracing::info!` so it doesn't also
    // land in bq_trace_logs.
    println!(
        "access status={} method={} url={} response_time_ms={} backend={} request_id={request_id}",
        response.get_status().as_u16(),
        capture.method,
        capture.url,
        response_time_ms.as_millis(),
        backend_name.unwrap_or("-"),
    );

    if !access_endpoint_configured() {
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
        response_time: response_time_ms,
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
    write_row(ACCESS_LOG_ENDPOINT, &row);
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

/// Write one trace-log row. Called from the `BqTraceLayer` in
/// `crate::logging` for every `tracing` event.
pub(crate) fn emit_trace_event(
    service_name: &str,
    request_id: Option<&str>,
    level: tracing::Level,
    message: &str,
    payload: Option<serde_json::Value>,
) {
    let row = TraceLog {
        timestamp: SystemTime::now(),
        service_name,
        request_id,
        level,
        message,
        payload: payload.as_ref(),
    };
    write_row(TRACE_LOG_ENDPOINT, &row);
}

fn write_row<T: Serialize>(endpoint_name: &str, row: &T) {
    let Ok(line) = serde_json::to_string(row) else {
        // Serialization can only fail for programming errors (e.g. a
        // map with non-string keys). Don't panic inside a request
        // handler over it.
        return;
    };
    // `writeln!` on a Fastly endpoint is effectively infallible; the
    // result is ignored to keep the call expression-level.
    let _ = writeln!(Endpoint::from_name(endpoint_name), "{line}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastly::http::Method;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::{net::Ipv4Addr, time::Duration};

    fn make_request() -> Request {
        Request::new(Method::GET, "https://example.thermondo.de/some/path")
    }

    #[test]
    fn access_log_serializes_all_populated_fields() {
        let row = AccessLog {
            timestamp: SystemTime::now(),
            service_name: "functions_proxy_prod",
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

        let v = serde_json::to_value(&row).expect("serialize");

        // Timestamp must be a JSON number for BQ TIMESTAMP coercion.
        assert!(v["timestamp"].is_number());
        // Numeric columns must not be quoted strings.
        assert!(v["status"].is_number());
        assert!(v["response_time_ms"].is_number());
        assert!(v["bytes_written"].is_number());

        // Key set must match the BQ schema column names exactly.
        let mut keys: Vec<_> = v
            .as_object()
            .unwrap()
            .keys()
            .map(ToOwned::to_owned)
            .collect();
        keys.sort();

        let expected = vec![
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
        ];

        assert_eq!(keys, expected);
    }

    #[test]
    fn access_log_drops_none_optional_fields() {
        let row = AccessLog {
            timestamp: UNIX_EPOCH,
            service_name: "svc",
            request_id: None,
            host: None,
            method: &http::Method::GET,
            url: &Url::parse("https://domain/").unwrap(),
            status: StatusCode::OK,
            response_time: Duration::from_millis(0),
            bytes_written: None,
            cache_status: None,
            client_ip: None,
            client_country: None,
            user_agent: None,
            referer: None,
            tls_protocol: None,
            backend_name: None,
            fastly_pop: None,
            fastly_server: None,
        };

        let v = serde_json::to_value(&row).expect("serialize");
        let obj = v.as_object().unwrap();

        // None optionals must be absent so BQ doesn't try to coerce a JSON null.
        for absent in [
            "request_id",
            "host",
            "bytes_written",
            "cache_status",
            "client_ip",
            "client_country",
            "user_agent",
            "referer",
            "tls_protocol",
            "backend_name",
            "fastly_pop",
            "fastly_server",
        ] {
            assert!(
                !obj.contains_key(absent),
                "expected `{absent}` to be omitted from serialized row"
            );
        }
        // Required columns stay even with default values.
        assert!(obj.contains_key("timestamp"));
        assert!(obj.contains_key("service_name"));
        assert!(obj.contains_key("method"));
        assert!(obj.contains_key("url"));
        assert!(obj.contains_key("status"));
        assert!(obj.contains_key("response_time_ms"));
    }

    #[test]
    fn trace_log_serializes_with_expected_columns() {
        let payload = json!({ "k": 1 });
        let row = TraceLog {
            timestamp: UNIX_EPOCH + Duration::from_millis(1500),
            service_name: "svc",
            request_id: Some("rid"),
            level: tracing::Level::INFO,
            message: "hi",
            payload: Some(&payload),
        };

        let v = serde_json::to_value(&row).expect("serialize");

        let mut keys = v
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort();

        assert_eq!(
            keys,
            vec![
                "level",
                "message",
                "payload",
                "request_id",
                "service_name",
                "timestamp",
            ]
        );

        // `payload` must serialize as a JSON-encoded *string*, not a
        // nested object: BQ's NDJSON load path for `JSON` columns
        // requires the value to be a string. A nested object causes
        // BQ to silently drop the row.
        let payload_str = v["payload"].as_str().expect("payload must be a string");
        let reparsed: serde_json::Value =
            serde_json::from_str(payload_str).expect("payload string must be valid JSON");
        assert_eq!(reparsed, json!({ "k": 1 }));
    }

    #[test]
    fn trace_log_drops_none_payload_and_request_id() {
        let row = TraceLog {
            timestamp: UNIX_EPOCH,
            service_name: "svc",
            request_id: None,
            level: tracing::Level::INFO,
            message: "",
            payload: None,
        };

        let v = serde_json::to_value(&row).expect("serialize");
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("request_id"));
        assert!(!obj.contains_key("payload"));
    }

    #[test]
    fn request_capture_minimal() {
        let req = make_request();
        let cap = RequestCapture::from(&req);

        assert_eq!(cap.method, "GET");
        assert_eq!(
            cap.url,
            Url::parse("https://example.thermondo.de/some/path").unwrap()
        );
        assert_eq!(cap.host(), Some("example.thermondo.de"));
        // No headers set, no Fastly geo lookup possible in tests.
        assert_eq!(cap.client_ip, None);
        assert_eq!(cap.client_country(), None);
        assert_eq!(cap.user_agent, None);
        assert_eq!(cap.referer, None);
    }

    #[test]
    fn request_capture_reads_fastly_client_ip_user_agent_referer() {
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5));

        let req = make_request()
            .with_header(FASTLY_CLIENT_IP, ip.to_string())
            .with_header("user-agent", "curl/8")
            .with_header("referer", "https://ref.example/");

        let cap = RequestCapture::from(&req);

        assert_eq!(cap.client_ip, Some(ip));
        assert_eq!(cap.user_agent.as_deref(), Some("curl/8"));
        assert_eq!(cap.referer.as_deref(), Some("https://ref.example/"));
        // geo_lookup needs Fastly runtime data; expect None in unit tests.
        assert_eq!(cap.client_country(), None);
    }

    #[test]
    fn request_capture_ignores_malformed_client_ip() {
        // A bogus header value should not derail capture; client_ip
        // parses to None (the column stores typed IpAddr) and country
        // is None as a consequence.
        let req = make_request().with_header(FASTLY_CLIENT_IP, "not-an-ip");
        let cap = RequestCapture::from(&req);

        assert_eq!(cap.client_ip, None);
        assert_eq!(cap.client_country(), None);
    }
}
