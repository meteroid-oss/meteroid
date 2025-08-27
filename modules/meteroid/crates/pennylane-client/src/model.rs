use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct QueryParams {
    #[serde(skip_serializing_if = "Option::is_none")]
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
