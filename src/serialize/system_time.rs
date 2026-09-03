use serde::Serializer;
use std::time::{SystemTime, UNIX_EPOCH};

/// Serialize a `SystemTime` as unix milliseconds (u128).
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

#[cfg(test)]
mod tests {}
