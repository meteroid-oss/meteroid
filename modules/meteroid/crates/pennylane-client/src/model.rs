use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(Debug, Serialize)]
pub struct QueryParams {
    #[serde(serialize_with = "as_json_string")]
    pub filter: Option<Vec<QueryFilter>>,
}

#[derive(Debug, Serialize)]
pub struct QueryFilter {
    pub field: String,
    pub operator: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct ListResponse<T> {
    #[allow(dead_code)]
    pub has_more: bool,
    pub items: Vec<T>,
    #[allow(dead_code)]
    pub next_cursor: Option<String>,
}

// ie: filter=[{"field":"external_reference","operator":"eq","value":"cus_xCX"}]
fn as_json_string<T, S>(value: &Option<Vec<T>>, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: serde::Serializer,
{
    if let Some(inner) = value {
        let json_str = serde_json::to_string(inner).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&json_str)
    } else {
        serializer.serialize_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_as_json_string() {
        let filter = Some(vec![QueryFilter {
            field: "external_reference".to_string(),
            operator: "eq".to_string(),
            value: "cus_xCX".to_string(),
        }]);

        let params = &QueryParams { filter };

        let value = serde_json::to_string(params).unwrap();

        assert_eq!(
            value,
            r#"{"filter":"[{\"field\":\"external_reference\",\"operator\":\"eq\",\"value\":\"cus_xCX\"}]"}"#
        )
    }
}
