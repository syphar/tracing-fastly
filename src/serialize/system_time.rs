use serde::Serializer;
use std::time::{SystemTime, UNIX_EPOCH};

/// Serialize a `SystemTime` as unix milliseconds (u128).
pub(crate) fn ser_unix_milliseconds<S>(
    timestamp: &SystemTime,
    serializer: S,
) -> Result<S::Ok, S::Error>
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
mod tests {
    use super::*;
    use serde::Serialize;
    use std::time::Duration;

    #[derive(Serialize)]
    struct Timestamp {
        #[serde(serialize_with = "ser_unix_milliseconds")]
        value: SystemTime,
    }

    #[test]
    fn serializes_unix_milliseconds() {
        let timestamp = Timestamp {
            value: UNIX_EPOCH + Duration::from_millis(1_500),
        };

        assert_eq!(
            serde_json::to_string(&timestamp).unwrap(),
            r#"{"value":1500}"#
        );
    }

    #[test]
    fn timestamps_before_the_epoch_fall_back_to_zero() {
        let timestamp = Timestamp {
            value: UNIX_EPOCH - Duration::from_millis(1),
        };

        assert_eq!(serde_json::to_string(&timestamp).unwrap(), r#"{"value":0}"#);
    }
}
