use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(Debug, Default, Serialize)]
pub(crate) struct ListQuery {
    /// A JSON array of `{field, operator, value}`, e.g.
    /// `[{"field":"external_reference","operator":"eq","value":"cus_xCX"}]`.
    pub filter: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

impl ListQuery {
    pub fn eq(field: &str, value: &str) -> Self {
        let filter = serde_json::json!([{ "field": field, "operator": "eq", "value": value }]);
        ListQuery {
            filter: Some(filter.to_string()),
            limit: Some(1),
            cursor: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListResponse<T> {
    pub items: Vec<T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_eq_filter_is_a_json_string() {
        let query = ListQuery::eq("external_reference", "cus_xCX");
        assert_eq!(
            serde_json::to_string(&query).unwrap(),
            r#"{"filter":"[{\"field\":\"external_reference\",\"operator\":\"eq\",\"value\":\"cus_xCX\"}]","limit":1}"#
        );
    }
}
