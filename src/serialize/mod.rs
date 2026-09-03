//! Collection of serde serialization helpers.
//!
//! Used by the different providers in the [`crate::providers`] module.
//!
//! Usable with `serialize_with`:
//!
//! ```rust
//! #[derive(Debug, Serialize)]
//! pub struct TraceLog {
//!    #[serde(serialize_with = "ser_unix_milliseconds")]
//!    pub timestamp: SystemTime,
//! }
//! ```

use serde::{Serialize, Serializer};
use std::{
    io::Write,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tracing_subscriber::fmt::MakeWriter;

/// Serializes one value as JSON followed by a newline.
pub fn write_ndjson_row<W, T>(writer: &W, row: &T) -> serde_json::Result<()>
where
    W: for<'a> MakeWriter<'a>,
    T: Serialize,
{
    let mut writer = writer.make_writer();
    serde_json::to_writer(&mut writer, row)?;
    writer.write_all(b"\n").map_err(serde_json::Error::io)
}

/// Serialize a `SystemTime` as unix seconds (float)
pub fn ser_unix_seconds<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    s.serialize_f64(secs)
}

/// Serialize a `SystemTime` as milliseconds.
pub fn ser_duration_ms<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(duration.as_secs_f64() * 1000.0)
}

pub fn ser_unix_milliseconds<S>(timestamp: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let milliseconds = timestamp
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    serializer.serialize_u128(milliseconds)
}

pub fn ser_http_status<S: Serializer>(
    status: &fastly::http::StatusCode,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_u16(status.as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use serde_json::json;
    use std::sync::Mutex;

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
    fn ndjson_row_has_a_trailing_newline() {
        let output = Mutex::new(Vec::new());

        write_ndjson_row(&output, &json!({ "message": "hello" })).unwrap();

        assert_eq!(&*output.lock().unwrap(), b"{\"message\":\"hello\"}\n");
    }
}
