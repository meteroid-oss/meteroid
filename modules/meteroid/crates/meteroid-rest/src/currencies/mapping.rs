use crate::currencies::model::Currency;
use meteroid_app::errors::RestApiError;
use std::str::FromStr;

pub fn from_str(input: &str) -> Result<Currency, RestApiError> {
    Currency::from_str(input).map_err(|_| RestApiError::StoreError)
}
