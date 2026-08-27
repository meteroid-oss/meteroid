//! EU seller allowlist — the single source of truth for which seller countries
//! may use the built-in EU VAT engine. Mirrors the EU customs-union membership in
//! `trade_agreements.json`. Add explicitly-supported near-EU sellers (e.g. Norway,
//! Switzerland) here as a one-line change.

/// ISO 3166-1 alpha-2 codes of the EU member states.
pub const EU_SELLER_COUNTRY_CODES: &[&str] = &[
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU", "IE", "IT",
    "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
];

/// Whether `code` (ISO 3166-1 alpha-2, case-insensitive) is an allowed EU seller.
pub fn is_eu_seller_country(code: &str) -> bool {
    EU_SELLER_COUNTRY_CODES
        .iter()
        .any(|c| c.eq_ignore_ascii_case(code))
}
