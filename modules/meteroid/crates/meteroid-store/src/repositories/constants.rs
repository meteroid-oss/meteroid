use crate::StoreResult;
use crate::constants::COUNTRIES;
use crate::errors::StoreError;
use crate::store::StoreInternal;

impl StoreInternal {
    pub fn get_currency_from_country(&self, country: &str) -> StoreResult<String> {
        let currency = COUNTRIES
            .iter()
            .find(|x| x.code == country)
            .map(|x| x.currency)
            .ok_or(StoreError::ValueNotFound(format!(
                "No currency found for country code {}",
                country
            )))?;
        Ok(currency.to_string())
    }
}
