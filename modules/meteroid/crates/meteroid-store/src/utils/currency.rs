use crate::StoreResult;
use crate::constants::COUNTRIES;
use crate::errors::StoreError;
use common_domain::country::CountryCode;

pub fn get_currency_from_country(country: &CountryCode) -> StoreResult<String> {
    let currency = COUNTRIES
        .iter()
        .find(|x| x.code == country.code)
        .map(|x| x.currency)
        .ok_or(StoreError::ValueNotFound(format!(
            "No currency found for country code {country}"
        )))?;
    Ok(currency.to_string())
}
