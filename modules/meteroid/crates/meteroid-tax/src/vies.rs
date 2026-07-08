//! Async VIES VAT verification.
//!
//! Hits the EU VIES REST API to confirm a VAT number is registered. VIES is
//! frequently unavailable (member-state systems go down independently), so every
//! failure mode that is not a definitive "not registered" answer is reported as
//! [`VatNumberExternalValidationResult::ServiceUnavailable`] — callers fail open.

use crate::model::{VatNumberExternalValidationResult, ViesCheckData, ViesValidation};
use std::time::Duration;

const VIES_BASE_URL: &str = "https://ec.europa.eu/taxation_customs/vies/rest-api/ms";
/// Qualified-check endpoint: unlike the unqualified `/ms/{cc}/vat/{n}` GET, it
/// accepts a requester identity and returns a consultation number.
const VIES_CHECK_URL: &str = "https://ec.europa.eu/taxation_customs/vies/rest-api/check-vat-number";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Member-state codes covered by VIES (EU-27, Greece as `EL`, plus Northern
/// Ireland `XI`). Notably excludes `GB`, `CH`, `NO`, which are not part of VIES.
const VIES_COUNTRIES: &[&str] = &[
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "EL", "ES", "FI", "FR", "HR", "HU", "IE", "IT",
    "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK", "XI",
];

/// Body of a qualified check (`POST /check-vat-number`). Field names must stay
/// camelCase to match the VIES contract.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckVatRequest<'a> {
    country_code: &'a str,
    vat_number: &'a str,
    requester_member_state_code: &'a str,
    requester_number: &'a str,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViesResponse {
    // The unqualified endpoint names this `isValid`, the qualified one `valid`.
    #[serde(alias = "valid")]
    is_valid: bool,
    #[serde(default)]
    user_error: Option<String>,
    #[serde(default)]
    request_date: Option<String>,
    // Consultation number — populated only by a qualified check.
    #[serde(default)]
    request_identifier: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    address: Option<String>,
}

/// VIES uses "---" (and sometimes empty strings) for withheld fields.
fn normalize_vies_field(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v != "---")
}

impl ViesResponse {
    fn check_data(&self) -> ViesCheckData {
        ViesCheckData {
            request_date: normalize_vies_field(self.request_date.clone()),
            request_identifier: normalize_vies_field(self.request_identifier.clone()),
            name: normalize_vies_field(self.name.clone()),
            address: normalize_vies_field(self.address.clone()),
        }
    }
}

lazy_static::lazy_static! {
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("Failed to build VIES HTTP client");
}

/// Validates a VAT number against VIES (unqualified). Returns `ServiceUnavailable`
/// (fail-open) for any non-VIES country, malformed input, transport error, or
/// VIES-side unavailability; only a clear VIES answer yields `Valid`/`Invalid`.
pub async fn validate(vat_number: &str) -> ViesValidation {
    validate_with_requester(vat_number, None).await
}

/// Validates `vat_number`, as a *qualified* check on behalf of `requester` (the
/// invoicing entity's own VAT number) when one is supplied and VIES-eligible. A
/// qualified check returns a consultation number (`request_identifier`) — legal
/// proof of the verification — that the unqualified check never provides. Falls
/// back to an unqualified check when no usable requester is available, or when
/// VIES rejects the requester's identity (the target check is unaffected by it).
pub async fn validate_with_requester(vat_number: &str, requester: Option<&str>) -> ViesValidation {
    let Some((country, number)) = split_vat_number(vat_number) else {
        return VatNumberExternalValidationResult::ServiceUnavailable.into();
    };

    if !VIES_COUNTRIES.contains(&country) {
        return VatNumberExternalValidationResult::ServiceUnavailable.into();
    }

    let requester = requester
        .and_then(split_vat_number)
        .filter(|(req_country, _)| VIES_COUNTRIES.contains(req_country));

    if let Some((req_country, req_number)) = requester
        && let Some(validation) = check_qualified(country, number, req_country, req_number).await
    {
        return validation;
    }

    check_unqualified(country, number).await
}

/// Qualified check via `POST /check-vat-number`. Returns `None` when VIES could
/// not authenticate the requester, signalling the caller to retry unqualified.
async fn check_qualified(
    country: &str,
    number: &str,
    req_country: &str,
    req_number: &str,
) -> Option<ViesValidation> {
    let request = CheckVatRequest {
        country_code: country,
        vat_number: number,
        requester_member_state_code: req_country,
        requester_number: req_number,
    };

    let response = match HTTP_CLIENT.post(VIES_CHECK_URL).json(&request).send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            log::warn!(
                "VIES qualified check returned status {} for {country}",
                response.status()
            );
            return Some(VatNumberExternalValidationResult::ServiceUnavailable.into());
        }
        Err(err) => {
            log::warn!("VIES qualified check failed for {country}: {err}");
            return Some(VatNumberExternalValidationResult::ServiceUnavailable.into());
        }
    };

    let body: ViesResponse = match response.json().await {
        Ok(body) => body,
        Err(err) => {
            log::warn!("VIES qualified response parse failed for {country}: {err}");
            return Some(VatNumberExternalValidationResult::ServiceUnavailable.into());
        }
    };

    // The requester's own identity was rejected; retry the target unqualified.
    if body.user_error.as_deref() == Some("INVALID_REQUESTER_INFO") {
        return None;
    }

    Some(interpret(body))
}

async fn check_unqualified(country: &str, number: &str) -> ViesValidation {
    let url = format!("{VIES_BASE_URL}/{country}/vat/{number}");

    let response = match HTTP_CLIENT.get(&url).send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            log::warn!("VIES returned status {} for {country}", response.status());
            return VatNumberExternalValidationResult::ServiceUnavailable.into();
        }
        Err(err) => {
            log::warn!("VIES request failed for {country}: {err}");
            return VatNumberExternalValidationResult::ServiceUnavailable.into();
        }
    };

    let body: ViesResponse = match response.json().await {
        Ok(body) => body,
        Err(err) => {
            log::warn!("VIES response parse failed for {country}: {err}");
            return VatNumberExternalValidationResult::ServiceUnavailable.into();
        }
    };

    interpret(body)
}

/// Maps a parsed VIES answer to a validation result. A definitive "not
/// registered" answer yields `Invalid`; anything else (MS_UNAVAILABLE, TIMEOUT,
/// rate limiting, ...) is `ServiceUnavailable` so we fail open.
fn interpret(body: ViesResponse) -> ViesValidation {
    if body.is_valid {
        return ViesValidation {
            result: VatNumberExternalValidationResult::Valid,
            check: Some(body.check_data()),
        };
    }

    match body.user_error.as_deref() {
        Some("INVALID") | Some("INVALID_INPUT") => ViesValidation {
            result: VatNumberExternalValidationResult::Invalid,
            check: Some(body.check_data()),
        },
        _ => VatNumberExternalValidationResult::ServiceUnavailable.into(),
    }
}

/// Whether a VAT number belongs to a country VIES can verify (EU-27 + `XI`).
/// Used to decide whether external validation is worth attempting at all.
pub fn is_vies_eligible(vat_number: &str) -> bool {
    match split_vat_number(vat_number) {
        Some((country, _)) => VIES_COUNTRIES.contains(&country),
        None => false,
    }
}

/// Splits a full VAT number into its 2-letter country prefix and the remainder.
fn split_vat_number(vat_number: &str) -> Option<(&str, &str)> {
    let trimmed = vat_number.trim();
    if trimmed.len() < 3 || !trimmed.is_char_boundary(2) {
        return None;
    }
    Some(trimmed.split_at(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_vat_number() {
        assert_eq!(
            split_vat_number("FR12345678901"),
            Some(("FR", "12345678901"))
        );
        assert_eq!(split_vat_number(" DE123456789 "), Some(("DE", "123456789")));
        assert_eq!(split_vat_number("FR"), None);
        assert_eq!(split_vat_number(""), None);
    }

    #[tokio::test]
    async fn test_non_vies_country_fails_open() {
        // GB is not part of VIES; must not error, must fail open.
        let validation = validate("GB123456789").await;
        assert!(matches!(
            validation.result,
            VatNumberExternalValidationResult::ServiceUnavailable
        ));
        assert!(validation.check.is_none());
    }

    #[test]
    fn test_normalize_vies_field() {
        assert_eq!(normalize_vies_field(Some("---".to_string())), None);
        assert_eq!(normalize_vies_field(Some("  ".to_string())), None);
        assert_eq!(normalize_vies_field(None), None);
        assert_eq!(
            normalize_vies_field(Some(" ACME SAS ".to_string())),
            Some("ACME SAS".to_string())
        );
    }

    #[test]
    fn test_check_request_uses_vies_camel_case() {
        let request = CheckVatRequest {
            country_code: "DE",
            vat_number: "123456789",
            requester_member_state_code: "FR",
            requester_number: "12345678901",
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["countryCode"], "DE");
        assert_eq!(json["vatNumber"], "123456789");
        assert_eq!(json["requesterMemberStateCode"], "FR");
        assert_eq!(json["requesterNumber"], "12345678901");
    }

    #[test]
    fn test_parses_qualified_response_valid_and_consultation_number() {
        // The qualified endpoint names the flag `valid` and returns a
        // requestIdentifier (consultation number).
        let body: ViesResponse = serde_json::from_str(
            r#"{"valid": true, "requestIdentifier": "WAPIAAAAX1", "name": "ACME SAS"}"#,
        )
        .unwrap();
        assert!(body.is_valid);
        let check = body.check_data();
        assert_eq!(check.request_identifier, Some("WAPIAAAAX1".to_string()));
        assert_eq!(check.name, Some("ACME SAS".to_string()));
    }

    #[test]
    fn test_parses_unqualified_response_is_valid_field() {
        let body: ViesResponse = serde_json::from_str(r#"{"isValid": false}"#).unwrap();
        assert!(!body.is_valid);
        assert!(body.check_data().request_identifier.is_none());
    }
}
