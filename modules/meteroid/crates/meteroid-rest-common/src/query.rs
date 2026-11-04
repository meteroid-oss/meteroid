use axum::extract::FromRequestParts;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use http::request::Parts;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, de};
use std::fmt;
use std::str::FromStr;

/// Serde deserialization decorator to map empty Strings to None,
pub fn empty_string_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => FromStr::from_str(s).map_err(de::Error::custom).map(Some),
    }
}

/// Custom Query extractor that uses serde_qs for proper array parameter handling.
/// This supports repeated query params like `status=A&status=B` being parsed as Vec.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueryParams<T>(pub T);

#[derive(Debug)]
pub struct QueryParamsRejection(String);

impl IntoResponse for QueryParamsRejection {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, self.0).into_response()
    }
}

impl<T, S> FromRequestParts<S> for QueryParams<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = QueryParamsRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default();
        // OpenAPI `style: form, explode: false` wire contract: SDKs emit
        // arrays as `?k=a,b,c`. Some SDKs percent-encode the separator
        // (`%2C`), so we treat both forms as list separators before handing
        // off to serde_html_form, which only understands repeated keys.
        let expanded = expand_csv_query_values(query);
        serde_html_form::from_str(&expanded)
            .map(QueryParams)
            .map_err(|e| QueryParamsRejection(format!("Failed to deserialize query string: {}", e)))
    }
}

/// Expands comma-joined array values in a URL-encoded query string into repeated-key form: `k=a,b,c` → `k=a&k=b&k=c`.
fn expand_csv_query_values(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut first = true;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        match pair.split_once('=') {
            Some((k, v)) => {
                let normalized = v.replace("%2C", ",").replace("%2c", ",");
                if !normalized.contains(',') {
                    if !first {
                        out.push('&');
                    }
                    first = false;
                    out.push_str(pair);
                    continue;
                }
                for piece in normalized.split(',') {
                    if piece.is_empty() {
                        continue;
                    }
                    if !first {
                        out.push('&');
                    }
                    first = false;
                    out.push_str(k);
                    out.push('=');
                    out.push_str(piece);
                }
            }
            None => {
                if !first {
                    out.push('&');
                }
                first = false;
                out.push_str(pair);
            }
        }
    }
    out
}

impl<T> axum_valid::HasValidate for QueryParams<T> {
    type Validate = T;
    fn get_validate(&self) -> &T {
        &self.0
    }
}

#[cfg(test)]
mod query_rewrite_tests {
    use super::expand_csv_query_values;

    #[test]
    fn passes_simple_pairs_through() {
        assert_eq!(expand_csv_query_values("a=1&b=2"), "a=1&b=2");
    }

    #[test]
    fn expands_csv_values() {
        assert_eq!(
            expand_csv_query_values("statuses=ACTIVE,PAUSED,TRIAL_ACTIVE"),
            "statuses=ACTIVE&statuses=PAUSED&statuses=TRIAL_ACTIVE"
        );
    }

    #[test]
    fn expands_only_the_csv_pair() {
        assert_eq!(
            expand_csv_query_values("page=0&statuses=A,B&per_page=10&statuses=C"),
            "page=0&statuses=A&statuses=B&per_page=10&statuses=C"
        );
    }

    #[test]
    fn preserves_already_repeated_keys() {
        assert_eq!(expand_csv_query_values("k=a&k=b"), "k=a&k=b");
    }

    #[test]
    fn preserves_empty_values() {
        assert_eq!(expand_csv_query_values("k="), "k=");
    }

    #[test]
    fn drops_empty_csv_pieces() {
        assert_eq!(expand_csv_query_values("k=a,,b,"), "k=a&k=b");
    }

    #[test]
    fn expands_mixed_encoded_and_literal_commas() {
        assert_eq!(
            expand_csv_query_values("statuses=ACTIVE%2CTRIAL_ACTIVE,PAUSED"),
            "statuses=ACTIVE&statuses=TRIAL_ACTIVE&statuses=PAUSED"
        );
    }

    #[test]
    fn empty_query() {
        assert_eq!(expand_csv_query_values(""), "");
    }
}
