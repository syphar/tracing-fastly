use serde::Serializer;
use std::time::{SystemTime, UNIX_EPOCH};

/// Serialize a `SystemTime` as unix seconds (float)
pub fn ser_unix_seconds<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    s.serialize_f64(secs)
}

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
mod tests {
    use super::*;
    use serde::Serialize;
    use std::time::Duration;

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
}
