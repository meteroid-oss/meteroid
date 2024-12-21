use crate::api_rest::currencies::model::Currency;
use crate::errors::RestApiError;

pub fn from_str(input: &str) -> Result<Currency, RestApiError> {
    serde_json::from_str(input).map_err(|_| RestApiError::StoreError)
}
