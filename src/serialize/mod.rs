//! Collection of serde serialization helpers.

pub mod system_time;

use serde::Serialize;
use std::{
    io::{self, Write},
    sync::Mutex,
};

/// A synchronized writer for structured NDJSON log records.
///
/// Each serialized value is passed to the underlying writer in exactly one
/// [`Write::write`] call. This makes the writer suitable for Fastly logging
/// endpoints, where each write represents one log record.
pub struct NdjsonWriter<W> {
    writer: Mutex<W>,
}

impl<W> NdjsonWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(writer),
        }
    }
}

impl<W> NdjsonWriter<W>
where
    W: Write,
{
    /// Serializes and writes one record.
    pub fn write<T>(&self, row: &T) -> serde_json::Result<()>
    where
        T: Serialize,
    {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| serde_json::Error::io(io::Error::other("NDJSON writer lock poisoned")))?;

        write_ndjson_row(&mut *writer, row)
    }
}

/// Serializes one NDJSON row and emits it with exactly one write.
///
/// Fastly logging endpoints treat every call to [`Write::write`] as one log
/// record, so this function does not append a newline. The writer supplies the
/// record boundary. A partial write is returned as [`io::ErrorKind::WriteZero`]
/// rather than retried, because retrying would create another Fastly log record.
pub fn write_ndjson_row<W, T>(writer: &mut W, row: &T) -> serde_json::Result<()>
where
    W: Write,
    T: Serialize,
{
    let json = serde_json::to_vec(row)?;
    match writer.write(&json) {
        Ok(written) if written == json.len() => Ok(()),
        Ok(written) => Err(serde_json::Error::io(io::Error::new(
            io::ErrorKind::WriteZero,
            format!(
                "incomplete log record: wrote {written} of {} bytes",
                json.len()
            ),
        ))),
        Err(error) => Err(serde_json::Error::io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::RecordWriter;
    use serde_json::json;

    #[test]
    fn ndjson_row_uses_one_write_without_a_newline() {
        let mut writer = RecordWriter::default();

        write_ndjson_row(&mut writer, &json!({ "message": "hello" })).unwrap();

        assert_eq!(
            *writer.records().unwrap(),
            [br#"{"message":"hello"}"#.to_vec()]
        );
    }

    #[test]
    fn synchronized_writer_emits_a_row() {
        let writer = NdjsonWriter::new(RecordWriter::default());

        writer.write(&json!({ "message": "hello" })).unwrap();

        assert_eq!(
            *writer.writer.lock().unwrap().records().unwrap(),
            [br#"{"message":"hello"}"#.to_vec()]
        );
    }
}
