use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Country {
    pub code: &'static str,
    pub name: &'static str,
    pub currency: &'static str,
    pub locale: &'static str,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Currency {
    pub code: &'static str,
    pub name: &'static str,
    pub symbol: &'static str,
    pub precision: u8,
}

const COUNTRIES_JSON: &str = include_str!("../static/countries.json");

pub static COUNTRIES: std::sync::LazyLock<&'static [Country]> = std::sync::LazyLock::new(|| {
    Box::leak(
        serde_json::from_str::<Vec<Country>>(COUNTRIES_JSON)
            .unwrap()
            .into_boxed_slice(),
    )
});

pub struct Countries {}
impl Countries {
    pub fn resolve_country(country: &str) -> Option<Country> {
        COUNTRIES.iter().find(|c| c.code == country).cloned()
    }
}

const CURRENCIES_JSON: &str = include_str!("../static/currencies.json");
pub static CURRENCIES: std::sync::LazyLock<&'static [Currency]> = std::sync::LazyLock::new(|| {
    Box::leak(
        serde_json::from_str::<Vec<Currency>>(CURRENCIES_JSON)
            .unwrap()
            .into_boxed_slice(),
    )
});

pub struct Currencies {}

impl Currencies {
    pub fn resolve_currency(currency: &str) -> Option<&Currency> {
        CURRENCIES.iter().find(|c| c.code == currency)
    }

    pub fn resolve_currency_precision(currency: &str) -> Option<u8> {
        Self::resolve_currency(currency).map(|c| c.precision)
    }
}
