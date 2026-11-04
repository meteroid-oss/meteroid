use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret, SecretString};

/// Standard Webhooks secret format.
pub const SECRET_PREFIX: &str = "whsec_";
const SECRET_BYTES: usize = 24;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebhookSecretError {
    #[error("webhook secret is not valid base64")]
    InvalidEncoding,
}

pub fn generate_secret() -> SecretString {
    let bytes: [u8; SECRET_BYTES] = rand::random();

    SecretString::from(format!("{SECRET_PREFIX}{}", STANDARD.encode(bytes)))
}

/// The HMAC key is the base64-decoded payload after the `whsec_` prefix. Secrets imported
/// from elsewhere may come without the prefix, so it is optional here.
pub fn decode_key(secret: &SecretString) -> Result<Vec<u8>, WebhookSecretError> {
    let raw = secret.expose_secret();
    let encoded = raw.strip_prefix(SECRET_PREFIX).unwrap_or(raw);

    STANDARD
        .decode(encoded)
        .map_err(|_| WebhookSecretError::InvalidEncoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_secrets_are_prefixed_and_24_bytes_long() {
        let secret = generate_secret();
        assert!(secret.expose_secret().starts_with(SECRET_PREFIX));
        assert_eq!(decode_key(&secret).unwrap().len(), SECRET_BYTES);
    }

    #[test]
    fn generated_secrets_are_unique() {
        let a = generate_secret();
        let b = generate_secret();
        assert_ne!(a.expose_secret(), b.expose_secret());
    }

    #[test]
    fn decode_key_accepts_a_secret_without_the_prefix() {
        let key = STANDARD.encode(b"0123456789abcdef01234567");
        let with = SecretString::from(format!("{SECRET_PREFIX}{key}"));
        let without = SecretString::from(key);

        assert_eq!(decode_key(&with).unwrap(), decode_key(&without).unwrap());
        assert_eq!(decode_key(&with).unwrap(), b"0123456789abcdef01234567");
    }

    #[test]
    fn decode_key_rejects_non_base64() {
        let secret = SecretString::from(format!("{SECRET_PREFIX}not base64 !!"));
        assert_eq!(
            decode_key(&secret).unwrap_err(),
            WebhookSecretError::InvalidEncoding
        );
    }
}
