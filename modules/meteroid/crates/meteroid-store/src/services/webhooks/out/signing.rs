use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use secrecy::SecretString;
use sha2::Sha256;

use super::secret::{WebhookSecretError, decode_key};

pub const HEADER_ID: &str = "webhook-id";
pub const HEADER_ID_LEGACY: &str = "svix-id";
pub const HEADER_TIMESTAMP: &str = "webhook-timestamp";
pub const HEADER_TIMESTAMP_LEGACY: &str = "svix-timestamp";
pub const HEADER_SIGNATURE: &str = "webhook-signature";
pub const HEADER_SIGNATURE_LEGACY: &str = "svix-signature";
pub const USER_AGENT: &str = "Meteroid-Webhooks/1.0";

/// `v1,<base64(HMAC_SHA256(key, "{id}.{timestamp}.{body}"))>`, per Standard Webhooks.
pub fn sign(
    secret: &SecretString,
    message_id: &str,
    timestamp: i64,
    body: &[u8],
) -> Result<String, WebhookSecretError> {
    let key = decode_key(secret)?;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(&key).map_err(|_| WebhookSecretError::InvalidEncoding)?;
    mac.update(message_id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);

    Ok(format!(
        "v1,{}",
        STANDARD.encode(mac.finalize().into_bytes())
    ))
}

/// Signatures for every still-valid secret, space-separated so a consumer that only knows
/// the pre-rotation secret keeps verifying during the grace window.
pub fn signature_header(
    secrets: &[SecretString],
    message_id: &str,
    timestamp: i64,
    body: &[u8],
) -> Result<String, WebhookSecretError> {
    let signatures = secrets
        .iter()
        .map(|secret| sign(secret, message_id, timestamp, body))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(signatures.join(" "))
}

/// The Standard Webhooks `webhook-*` headers. `svix_headers` adds the same values under
/// Svix's `svix-*` names, for an endpoint migrated from Svix whose receiver may read them.
pub fn build_headers(
    secrets: &[SecretString],
    message_id: &str,
    timestamp: i64,
    body: &[u8],
    svix_headers: bool,
) -> Result<Vec<(&'static str, String)>, WebhookSecretError> {
    let signature = signature_header(secrets, message_id, timestamp, body)?;
    let timestamp = timestamp.to_string();

    let mut headers = vec![
        (HEADER_ID, message_id.to_string()),
        (HEADER_TIMESTAMP, timestamp.clone()),
        (HEADER_SIGNATURE, signature.clone()),
        ("content-type", "application/json".to_string()),
        ("user-agent", USER_AGENT.to_string()),
    ];
    if svix_headers {
        headers.extend([
            (HEADER_ID_LEGACY, message_id.to_string()),
            (HEADER_TIMESTAMP_LEGACY, timestamp),
            (HEADER_SIGNATURE_LEGACY, signature),
        ]);
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::webhooks::out::secret::SECRET_PREFIX;

    fn secret(raw: &[u8]) -> SecretString {
        SecretString::from(format!("{SECRET_PREFIX}{}", STANDARD.encode(raw)))
    }

    /// Independent re-implementation of the signature, so the test fails if `sign` drifts.
    fn expected(key: &[u8], message_id: &str, timestamp: i64, body: &[u8]) -> String {
        let signed = format!(
            "{message_id}.{timestamp}.{}",
            std::str::from_utf8(body).unwrap()
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
        mac.update(signed.as_bytes());
        format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn signature_matches_an_independent_hmac() {
        let key = b"0123456789abcdef01234567";
        let body = br#"{"id":"evt_1"}"#;

        assert_eq!(
            sign(&secret(key), "evt_1", 1_700_000_000, body).unwrap(),
            expected(key, "evt_1", 1_700_000_000, body)
        );
    }

    #[test]
    fn signature_changes_with_id_timestamp_and_body() {
        let s = secret(b"0123456789abcdef01234567");
        let base = sign(&s, "evt_1", 1_700_000_000, b"{}").unwrap();

        assert_ne!(base, sign(&s, "evt_2", 1_700_000_000, b"{}").unwrap());
        assert_ne!(base, sign(&s, "evt_1", 1_700_000_001, b"{}").unwrap());
        assert_ne!(
            base,
            sign(&s, "evt_1", 1_700_000_000, b"{\"a\":1}").unwrap()
        );
    }

    #[test]
    fn rotation_produces_two_space_separated_signatures() {
        let current = secret(b"0123456789abcdef01234567");
        let previous = secret(b"76543210fedcba9876543210");
        let body = b"{}";

        let header =
            signature_header(&[current.clone(), previous.clone()], "evt_1", 42, body).unwrap();
        let parts: Vec<&str> = header.split(' ').collect();

        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], sign(&current, "evt_1", 42, body).unwrap());
        assert_eq!(parts[1], sign(&previous, "evt_1", 42, body).unwrap());
    }

    #[test]
    fn only_svix_endpoints_get_the_svix_spelling() {
        let headers = build_headers(
            &[secret(b"0123456789abcdef01234567")],
            "evt_1",
            42,
            b"{}",
            false,
        )
        .unwrap();

        assert!(headers.iter().any(|(k, _)| *k == HEADER_SIGNATURE));
        assert!(!headers.iter().any(|(k, _)| {
            [
                HEADER_ID_LEGACY,
                HEADER_TIMESTAMP_LEGACY,
                HEADER_SIGNATURE_LEGACY,
            ]
            .contains(k)
        }));
    }

    #[test]
    fn headers_carry_both_spellings_with_identical_values() {
        let headers = build_headers(
            &[secret(b"0123456789abcdef01234567")],
            "evt_1",
            42,
            b"{}",
            true,
        )
        .unwrap();

        let get = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };

        assert_eq!(get(HEADER_ID), "evt_1");
        assert_eq!(get(HEADER_ID_LEGACY), get(HEADER_ID));
        assert_eq!(get(HEADER_TIMESTAMP), "42");
        assert_eq!(get(HEADER_TIMESTAMP_LEGACY), get(HEADER_TIMESTAMP));
        assert_eq!(get(HEADER_SIGNATURE_LEGACY), get(HEADER_SIGNATURE));
        assert_eq!(get("content-type"), "application/json");
        assert_eq!(get("user-agent"), USER_AGENT);
    }

    #[test]
    fn an_invalid_secret_is_rejected() {
        let bad = SecretString::from(format!("{SECRET_PREFIX}not base64 !!"));
        assert_eq!(
            sign(&bad, "evt_1", 42, b"{}").unwrap_err(),
            WebhookSecretError::InvalidEncoding
        );
    }
}
