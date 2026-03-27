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

pub const EURO_COUNTRIES: [rust_iso3166::CountryCode; 27] = [
    rust_iso3166::DE, // Germany
    rust_iso3166::AT, // Austria
    rust_iso3166::BE, // Belgium
    rust_iso3166::BG, // Bulgaria
    rust_iso3166::CY, // Cyprus
    rust_iso3166::HR, // Croatia
    rust_iso3166::DK, // Denmark
    rust_iso3166::ES, // Spain
    rust_iso3166::EE, // Estonia
    rust_iso3166::FI, // Finland
    rust_iso3166::FR, // France
    rust_iso3166::GR, // Greece
    rust_iso3166::HU, // Hungary
    rust_iso3166::IE, // Ireland
    rust_iso3166::IT, // Italy
    rust_iso3166::LV, // Latvia
    rust_iso3166::LT, // Lithuania
    rust_iso3166::LU, // Luxembourg
    rust_iso3166::MT, // Malta
    rust_iso3166::NL, // Netherlands
    rust_iso3166::PL, // Poland
    rust_iso3166::PT, // Portugal
    rust_iso3166::CZ, // Czech Republic
    rust_iso3166::RO, // Romania
    rust_iso3166::SK, // Slovakia
    rust_iso3166::SI, // Slovenia
    rust_iso3166::SE, // Sweden
];
