//! Async VIES VAT verification.
//!
//! Hits the EU VIES REST API to confirm a VAT number is registered. VIES is
//! frequently unavailable (member-state systems go down independently), so every
//! failure mode that is not a definitive "not registered" answer is reported as
//! [`VatNumberExternalValidationResult::ServiceUnavailable`] — callers fail open.

use crate::model::{VatNumberExternalValidationResult, ViesCheckData, ViesValidation};
use std::time::{Duration, Instant};

const VIES_BASE_URL: &str = "https://ec.europa.eu/taxation_customs/vies/rest-api/ms";
/// Qualified-check endpoint: unlike the unqualified `/ms/{cc}/vat/{n}` GET, it
/// accepts a requester identity and returns a consultation number.
const VIES_CHECK_URL: &str = "https://ec.europa.eu/taxation_customs/vies/rest-api/check-vat-number";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Minimum spacing between VIES requests. VIES rejects bursts (MS_MAX_CONCURRENT_REQ
/// / GLOBAL_MAX_CONCURRENT_REQ), so all requests are serialized through a
/// process-wide gate and paced to at most one every `MIN_REQUEST_INTERVAL`.
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(500);

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

/// One entry of the qualified endpoint's error envelope
/// (`{"actionSucceed": false, "errorWrappers": [{"error": "..."}]}`), which
/// carries no `valid` field at all.
#[derive(serde::Deserialize)]
struct ViesErrorWrapper {
    error: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViesResponse {
    // Present on any definitive answer — `isValid` unqualified, `valid` qualified.
    // Absent on qualified *error* responses, hence `Option`.
    #[serde(default, alias = "valid")]
    is_valid: Option<bool>,
    // Status/error code on the unqualified endpoint (`VALID`, `INVALID`, ...).
    #[serde(default)]
    user_error: Option<String>,
    // The qualified endpoint reports errors here instead, with no `valid` field.
    #[serde(default)]
    error_wrappers: Vec<ViesErrorWrapper>,
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

    /// The qualified endpoint's error code, when this is an error envelope.
    fn first_error(&self) -> Option<&str> {
        self.error_wrappers.first().map(|w| w.error.as_str())
    }

    fn with_result(&self, result: VatNumberExternalValidationResult) -> ViesValidation {
        ViesValidation {
            result,
            check: Some(self.check_data()),
        }
    }
}

lazy_static::lazy_static! {
    static ref HTTP_CLIENT: reqwest::Client = http_client_tracing::client_builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("Failed to build VIES HTTP client");

    /// Serializes VIES calls process-wide and tracks the last dispatch, so bursts
    /// (a revalidation batch, or many parallel workers) can't trip VIES's
    /// concurrency limits. Holds `None` until the first request.
    static ref REQUEST_GATE: tokio::sync::Mutex<Option<Instant>> =
        tokio::sync::Mutex::new(None);
}

/// Acquires the process-wide VIES gate: waits its turn (one request in flight at
/// a time), then sleeps until `MIN_REQUEST_INTERVAL` has elapsed since the last
/// dispatch. The returned guard must be held for the whole request/response.
async fn throttle() -> tokio::sync::MutexGuard<'static, Option<Instant>> {
    let mut last = REQUEST_GATE.lock().await;
    if let Some(previous) = *last
        && let Some(wait) = MIN_REQUEST_INTERVAL.checked_sub(previous.elapsed())
    {
        tokio::time::sleep(wait).await;
    }
    *last = Some(Instant::now());
    last
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

    let _slot = throttle().await;
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

    let Some(body) = parse_vies_response(response, country, "qualified").await else {
        return Some(VatNumberExternalValidationResult::ServiceUnavailable.into());
    };

    // The requester's own identity was rejected; retry the target unqualified.
    if body.first_error() == Some("INVALID_REQUESTER_INFO") {
        return None;
    }

    Some(interpret(body))
}

async fn check_unqualified(country: &str, number: &str) -> ViesValidation {
    let url = format!("{VIES_BASE_URL}/{country}/vat/{number}");

    let _slot = throttle().await;
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

    let Some(body) = parse_vies_response(response, country, "unqualified").await else {
        return VatNumberExternalValidationResult::ServiceUnavailable.into();
    };

    interpret(body)
}

/// Reads and JSON-parses a VIES body, logging the raw payload (truncated) on
/// failure — VIES parse errors are usually an HTML error page, an empty body, or
/// a rate-limit notice, which reqwest's bare "error decoding response body" hides.
async fn parse_vies_response(
    response: reqwest::Response,
    country: &str,
    kind: &str,
) -> Option<ViesResponse> {
    let body = match response.text().await {
        Ok(body) => body,
        Err(err) => {
            log::warn!("VIES {kind} response read failed for {country}: {err}");
            return None;
        }
    };

    match serde_json::from_str::<ViesResponse>(&body) {
        Ok(parsed) => Some(parsed),
        Err(err) => {
            log::warn!(
                "VIES {kind} response parse failed for {country}: {err} (body: {})",
                truncate_for_log(&body)
            );
            None
        }
    }
}

/// Caps a body for logging: VIES payloads are tiny, but an upstream error page
/// can be large, so one bad response must not flood the logs.
fn truncate_for_log(body: &str) -> String {
    const MAX: usize = 512;
    let body = body.trim();
    if body.len() <= MAX {
        return body.to_string();
    }
    let mut end = MAX;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… (truncated, {} bytes total)", &body[..end], body.len())
}

/// Maps a parsed VIES answer to a validation result. A definitive "not
/// registered" answer yields `Invalid`; a service-side error (MS_UNAVAILABLE,
/// TIMEOUT, rate limiting, requester rejection, ...) yields `ServiceUnavailable`
/// so we fail open. The two endpoints disagree on shape, so both are handled:
/// the qualified endpoint reports errors via `errorWrappers` (no `valid`) and a
/// definitive negative as a bare `valid:false`; the unqualified one keeps
/// `isValid` alongside a `userError` code.
fn interpret(body: ViesResponse) -> ViesValidation {
    if let Some(code) = body.first_error() {
        return match code {
            "INVALID_INPUT" => body.with_result(VatNumberExternalValidationResult::Invalid),
            _ => VatNumberExternalValidationResult::ServiceUnavailable.into(),
        };
    }

    match body.is_valid {
        Some(true) => body.with_result(VatNumberExternalValidationResult::Valid),
        // A definitive `INVALID`, or the qualified endpoint's bare `valid:false`
        // (no code). Any other code (MS_UNAVAILABLE, ...) fails open.
        Some(false) => match body.user_error.as_deref() {
            Some("INVALID") | Some("INVALID_INPUT") | None => {
                body.with_result(VatNumberExternalValidationResult::Invalid)
            }
            Some(_) => VatNumberExternalValidationResult::ServiceUnavailable.into(),
        },
        None => VatNumberExternalValidationResult::ServiceUnavailable.into(),
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
        assert_eq!(body.is_valid, Some(true));
        let check = body.check_data();
        assert_eq!(check.request_identifier, Some("WAPIAAAAX1".to_string()));
        assert_eq!(check.name, Some("ACME SAS".to_string()));
    }

    #[test]
    fn test_parses_unqualified_response_is_valid_field() {
        let body: ViesResponse = serde_json::from_str(r#"{"isValid": false}"#).unwrap();
        assert_eq!(body.is_valid, Some(false));
        assert!(body.check_data().request_identifier.is_none());
    }

    fn parse(json: &str) -> ViesResponse {
        serde_json::from_str(json).expect("VIES body should parse")
    }

    #[test]
    fn test_parses_qualified_error_envelope() {
        // The qualified endpoint's error shape carries no `valid` field; it must
        // still parse (previously this failed to decode) and expose the code.
        let body = parse(
            r#"{"actionSucceed": false, "errorWrappers": [{"error": "INVALID_REQUESTER_INFO"}]}"#,
        );
        assert_eq!(body.is_valid, None);
        assert_eq!(body.first_error(), Some("INVALID_REQUESTER_INFO"));
    }

    #[test]
    fn test_interpret_qualified_valid() {
        let result = interpret(parse(
            r#"{"valid": true, "requestIdentifier": "WAPIAAAAX1"}"#,
        ));
        assert!(matches!(
            result.result,
            VatNumberExternalValidationResult::Valid
        ));
    }

    #[test]
    fn test_interpret_qualified_bare_false_is_invalid() {
        // Qualified endpoint signals "not registered" as a bare `valid:false`,
        // with no userError — this must be Invalid, not ServiceUnavailable.
        let result = interpret(parse(r#"{"valid": false}"#));
        assert!(matches!(
            result.result,
            VatNumberExternalValidationResult::Invalid
        ));
    }

    #[test]
    fn test_interpret_unqualified_invalid_and_valid() {
        assert!(matches!(
            interpret(parse(r#"{"isValid": false, "userError": "INVALID"}"#)).result,
            VatNumberExternalValidationResult::Invalid
        ));
        assert!(matches!(
            interpret(parse(r#"{"isValid": true, "userError": "VALID"}"#)).result,
            VatNumberExternalValidationResult::Valid
        ));
    }

    #[test]
    fn test_interpret_service_errors_fail_open() {
        // Both endpoints' service-side errors must fail open.
        for body in [
            r#"{"isValid": false, "userError": "MS_UNAVAILABLE"}"#,
            r#"{"actionSucceed": false, "errorWrappers": [{"error": "MS_MAX_CONCURRENT_REQ"}]}"#,
            r#"{"actionSucceed": false, "errorWrappers": [{"error": "INVALID_REQUESTER_INFO"}]}"#,
            r#"{}"#,
        ] {
            assert!(
                matches!(
                    interpret(parse(body)).result,
                    VatNumberExternalValidationResult::ServiceUnavailable
                ),
                "expected ServiceUnavailable for {body}"
            );
        }
    }

    #[test]
    fn test_interpret_real_qualified_payloads() {
        // Verbatim live qualified-endpoint bodies (trader fields included) for a
        // registered vs an unregistered but well-formed FR number.
        let valid = interpret(parse(
            r#"{"countryCode":"FR","vatNumber":"65793569757","requestDate":"2026-07-10T16:32:22.920Z","valid":true,"requestIdentifier":"WAPIAAAAZ9M33keA","name":"SAS LITTLE BIG CONNECTION","address":"7 RUE DE CHATEAUDUN\n75009 PARIS","traderName":"---","traderNameMatch":"NOT_PROCESSED"}"#,
        ));
        assert!(matches!(
            valid.result,
            VatNumberExternalValidationResult::Valid
        ));
        assert_eq!(
            valid.check.and_then(|c| c.name),
            Some("SAS LITTLE BIG CONNECTION".to_string())
        );

        let invalid = interpret(parse(
            r#"{"countryCode":"FR","vatNumber":"21349824531","requestDate":"2026-07-10T16:34:26.272Z","valid":false,"requestIdentifier":"","name":"---","address":"---","traderNameMatch":"NOT_PROCESSED"}"#,
        ));
        assert!(matches!(
            invalid.result,
            VatNumberExternalValidationResult::Invalid
        ));
    }

    #[test]
    fn test_interpret_invalid_input_error_is_invalid() {
        let result = interpret(parse(
            r#"{"actionSucceed": false, "errorWrappers": [{"error": "INVALID_INPUT"}]}"#,
        ));
        assert!(matches!(
            result.result,
            VatNumberExternalValidationResult::Invalid
        ));
    }

    #[test]
    fn test_truncate_for_log_trims_short_bodies() {
        assert_eq!(truncate_for_log("  {\"valid\":true}\n"), "{\"valid\":true}");
    }

    #[tokio::test]
    async fn test_throttle_paces_consecutive_requests() {
        // Three back-to-back acquisitions: the first is immediate, the next two
        // each wait a full interval, so the batch spans at least two intervals.
        let start = Instant::now();
        for _ in 0..3 {
            let _slot = throttle().await;
        }
        assert!(start.elapsed() >= MIN_REQUEST_INTERVAL * 2);
    }

    #[test]
    fn test_truncate_for_log_caps_long_bodies() {
        let body = "x".repeat(1000);
        let out = truncate_for_log(&body);
        assert!(out.starts_with(&"x".repeat(512)));
        assert!(out.contains("truncated, 1000 bytes total"));
        assert!(out.len() < body.len());
    }
}
