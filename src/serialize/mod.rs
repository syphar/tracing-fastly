//! Collection of serde serialization helpers.

pub(crate) mod system_time;

use serde::Serialize;
use std::io::{self, Write};

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
    use serde_json::json;

    #[derive(Default)]
    struct RecordWriter(Vec<Vec<u8>>);

    impl Write for RecordWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.push(bytes.to_vec());
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn ndjson_row_uses_one_write_without_a_newline() {
        let mut writer = RecordWriter::default();

        write_ndjson_row(&mut writer, &json!({ "message": "hello" })).unwrap();

        assert_eq!(writer.0, [br#"{"message":"hello"}"#.to_vec()]);
    }
}
