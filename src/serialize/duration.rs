use serde::Serializer;
use std::time::Duration;

/// Serialize a `Duration` as milliseconds.
pub fn ser_milliseconds<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u128(duration.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[test]
    fn duration_serializes_as_millis() {
        #[derive(Serialize)]
        struct Row {
            #[serde(serialize_with = "ser_milliseconds")]
            d: Duration,
        }
        let v = serde_json::to_value(Row {
            d: Duration::from_millis(12),
        })
        .unwrap();
        assert_eq!(v["d"].as_f64().unwrap(), 12.0);
    }
}
