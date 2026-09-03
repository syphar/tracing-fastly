use serde::Serializer;

pub fn ser_status<S: Serializer>(
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

    #[test]
    fn status_serializes_as_u16() {
        #[derive(Serialize)]
        struct Row {
            #[serde(serialize_with = "ser_status")]
            s: fastly::http::StatusCode,
        }
        let v = serde_json::to_value(Row {
            s: fastly::http::StatusCode::NOT_FOUND,
        })
        .unwrap();
        assert_eq!(v["s"], json!(404));
        assert!(v["s"].is_number());
    }
}
