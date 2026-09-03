//! Utilities for testing custom structured-event providers.

use std::{
    io::{self, Write},
    sync::{Arc, LockResult, Mutex, MutexGuard},
};

/// A cloneable writer that captures every [`Write::write`] call as a record.
///
/// Clones share the same collection, so a writer can be moved into a provider
/// while the test keeps another handle for assertions.
#[derive(Clone, Default)]
pub struct RecordWriter {
    records: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl RecordWriter {
    /// Returns the captured writes in the order they occurred.
    pub fn records(&self) -> LockResult<MutexGuard<'_, Vec<Vec<u8>>>> {
        self.records.lock()
    }
}

impl Write for RecordWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.records
            .lock()
            .map_err(|_| io::Error::other("record writer lock poisoned"))?
            .push(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_captured_writes() {
        let writer = RecordWriter::default();
        let mut clone = writer.clone();

        clone.write_all(b"record").unwrap();

        assert_eq!(*writer.records().unwrap(), [b"record".to_vec()]);
    }
}
