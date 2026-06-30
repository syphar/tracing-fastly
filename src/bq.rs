//! BigQuery-over-Fastly helpers.
//!
//! Fastly ships a named `logging_bigquery` endpoint's lines as
//! newline-delimited JSON, which BigQuery ingests via load jobs. The
//! serialized JSON *is* the row: each top-level key is a column and value
//! types must coerce to the column types. That coercion has sharp edges —
//! this module captures them so you don't rediscover each one by watching
//! rows silently vanish.
//!
//! You bring the row: a `#[derive(Serialize)]` struct whose fields are your
//! columns (kept in lockstep with your BigQuery table schema), using the
//! `ser_*` functions below via `#[serde(serialize_with = ...)]`. Then
//! [`write_ndjson_row`] ships it. See `examples/bigquery.rs`.

use fastly::log::Endpoint;
use serde::{Serialize, Serializer, ser::Error as _};
use std::{io::Write, time::Duration};

/// Whether a Fastly log endpoint with this name is configured on the service.
///
/// Writing to an unconfigured endpoint is silently dropped at the edge, so
/// this is only needed to skip building an expensive row — emitting
/// unconditionally is safe.
pub fn endpoint_configured(name: &str) -> bool {
    Endpoint::try_from_name(name).is_ok()
}

/// Serialize `row` to one NDJSON line and write it to the named Fastly log
/// endpoint. Fire-and-forget: serialization errors (only possible for
/// programming mistakes, e.g. a map with non-string keys) and the
/// effectively-infallible endpoint write are both swallowed, so a broken sink
/// can never break the request being served.
pub fn write_ndjson_row<T: Serialize>(endpoint: &str, row: &T) {
    let Ok(line) = serde_json::to_string(row) else {
        return;
    };
    let _ = writeln!(Endpoint::from_name(endpoint), "{line}");
}

/// Serialize a [`SystemTime`](std::time::SystemTime) as Unix epoch seconds (a
/// JSON number) so a BigQuery `TIMESTAMP` column coerces it. A pre-epoch value
/// falls back to `0.0` rather than failing the row.
pub fn ser_unix_seconds<S: Serializer>(
    t: &std::time::SystemTime,
    s: S,
) -> Result<S::Ok, S::Error> {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    s.serialize_f64(secs)
}

/// Serialize a [`Duration`] as milliseconds (a JSON number / `FLOAT64`).
pub fn ser_duration_ms<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(duration.as_secs_f64() * 1000.0)
}

/// Serialize an HTTP status as its numeric code (`INTEGER`), not the quoted
/// reason phrase.
pub fn ser_http_status<S: Serializer>(
    status: &fastly::http::StatusCode,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_u16(status.as_u16())
}

/// Serialize an optional value as a **JSON-encoded string** for a BigQuery
/// `JSON` column.
///
/// BigQuery's NDJSON load path expects a `JSON` column's value to be a string
/// containing JSON, *not* a nested object — a nested object type-mismatches
/// and BigQuery silently drops the whole row. `None` serializes as absent
/// (pair with `#[serde(skip_serializing_if = "Option::is_none")]` or
/// `skip_serializing_none`).
pub fn ser_json_as_string<T, S>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: Serializer,
{
    match v {
        Some(val) => {
            let encoded = serde_json::to_string(val).map_err(S::Error::custom)?;
            s.serialize_str(&encoded)
        }
        None => s.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn unix_seconds_is_a_number() {
        #[derive(Serialize)]
        struct Row {
            #[serde(serialize_with = "ser_unix_seconds")]
            t: SystemTime,
        }
        let v = serde_json::to_value(Row {
            t: UNIX_EPOCH + Duration::from_millis(1500),
        })
        .unwrap();
        assert!(v["t"].is_number());
        assert_eq!(v["t"].as_f64().unwrap(), 1.5);
    }

    #[test]
    fn pre_epoch_falls_back_to_zero() {
        #[derive(Serialize)]
        struct Row {
            #[serde(serialize_with = "ser_unix_seconds")]
            t: SystemTime,
        }
        let v = serde_json::to_value(Row {
            t: UNIX_EPOCH - Duration::from_secs(10),
        })
        .unwrap();
        assert_eq!(v["t"].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn duration_serializes_as_millis() {
        #[derive(Serialize)]
        struct Row {
            #[serde(serialize_with = "ser_duration_ms")]
            d: Duration,
        }
        let v = serde_json::to_value(Row {
            d: Duration::from_millis(12),
        })
        .unwrap();
        assert_eq!(v["d"].as_f64().unwrap(), 12.0);
    }

    #[test]
    fn status_serializes_as_u16() {
        #[derive(Serialize)]
        struct Row {
            #[serde(serialize_with = "ser_http_status")]
            s: fastly::http::StatusCode,
        }
        let v = serde_json::to_value(Row {
            s: fastly::http::StatusCode::NOT_FOUND,
        })
        .unwrap();
        assert_eq!(v["s"], json!(404));
        assert!(v["s"].is_number());
    }

    #[test]
    fn json_payload_is_a_string_not_an_object() {
        #[derive(Serialize)]
        struct Row<'a> {
            #[serde(serialize_with = "ser_json_as_string")]
            p: Option<&'a serde_json::Value>,
        }
        let payload = json!({ "k": 1 });
        let v = serde_json::to_value(Row { p: Some(&payload) }).unwrap();

        // Must be a JSON-encoded string, and round-trip back to the object.
        let s = v["p"].as_str().expect("payload must be a string");
        assert_eq!(serde_json::from_str::<serde_json::Value>(s).unwrap(), json!({ "k": 1 }));
    }

    #[test]
    fn json_payload_none_serializes_as_null() {
        #[derive(Serialize)]
        struct Row<'a> {
            #[serde(serialize_with = "ser_json_as_string")]
            p: Option<&'a serde_json::Value>,
        }
        // Without skip_serializing_if, None serializes as JSON null; with the
        // skip attribute (see the example) the key is omitted entirely.
        let v = serde_json::to_value(Row { p: None }).unwrap();
        assert!(v["p"].is_null());
    }
}
