//! Collection of serde serialization helpers.
//!
//! Used by the different providers in the [`crate::providers`] module.
//!
//! Usable with `serialize_with`:
//!
//! ```rust
//! #[derive(Debug, Serialize)]
//! pub struct TraceLog {
//!    #[serde(serialize_with = "system_time::ser_unix_milliseconds")]
//!    pub timestamp: SystemTime,
//! }
//! ```

pub mod duration;
pub mod system_time;

use serde::Serialize;
use std::io::Write;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[test]
    fn ndjson_row_has_a_trailing_newline() {
        let output = Mutex::new(Vec::new());

        write_ndjson_row(&output, &json!({ "message": "hello" })).unwrap();

        assert_eq!(&*output.lock().unwrap(), b"{\"message\":\"hello\"}\n");
    }
}
