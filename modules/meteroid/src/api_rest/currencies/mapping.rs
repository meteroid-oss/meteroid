use crate::api_rest::currencies::model::Currency;
use crate::errors::RestApiError;
use std::str::FromStr;

pub fn from_str(input: &str) -> Result<Currency, RestApiError> {
    Currency::from_str(input).map_err(|_| RestApiError::StoreError)
}
