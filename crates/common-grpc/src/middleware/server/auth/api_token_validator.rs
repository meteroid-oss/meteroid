use anyhow::anyhow;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use sha2::{Digest, Sha256};

/// Domain separator, so this digest can never be confused with another SHA-256 use.
const FINGERPRINT_DOMAIN: &[u8] = b"meteroid:api-token-cache:v1";

/// Digest of a complete presented API key, for use as an authorization-cache key.
///
/// An authorization cache keyed by token id alone lets any secret carrying a known id ride a
/// warm entry. Keying on this digest instead binds a cache hit to the exact credential that
/// passed Argon2 verification, without retaining the plaintext secret as a key.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialFingerprint([u8; 32]);

impl std::fmt::Debug for CredentialFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The digest is secret-derived; rendering it would leak it into logs and spans.
        f.write_str("CredentialFingerprint(redacted)")
    }
}

pub struct ApiTokenValidator {
    id_part: String,
    hash_part: String,
}

impl ApiTokenValidator {
    pub fn parse_api_key(api_key: &str) -> Result<Self, anyhow::Error> {
        let parts = api_key.rsplitn(2, '/').collect::<Vec<&str>>();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid API key format."));
        }

        let id_part = parts[0];
        let hash_part = parts[1]
            .rsplit('_')
            .nth(0)
            .ok_or(anyhow!("Invalid API key format."))?;

        Ok(Self {
            id_part: id_part.to_string(),
            hash_part: hash_part.to_string(),
        })
    }

    pub fn extract_identifier(&self) -> Result<uuid::Uuid, anyhow::Error> {
        // Decode the identifier from base62 to UUID
        let id_u128 =
            base62::decode(&self.id_part).map_err(|_| anyhow!("Failed to decode identifier"))?;
        Ok(uuid::Uuid::from_u128(id_u128))
    }

    /// Binds the token identity used for the store lookup to the exact secret presented.
    ///
    /// Built from the decoded id rather than its textual form so that equivalent base62
    /// encodings share one cache entry, and length-framed so that no two distinct
    /// id/secret pairs can produce the same digest.
    pub fn credential_fingerprint(&self, id: &uuid::Uuid) -> CredentialFingerprint {
        let secret = self.hash_part.as_bytes();

        let mut hasher = Sha256::new();
        hasher.update(FINGERPRINT_DOMAIN);
        hasher.update(id.as_bytes());
        hasher.update((secret.len() as u64).to_le_bytes());
        hasher.update(secret);

        CredentialFingerprint(hasher.finalize().into())
    }

    pub fn validate_hash(&self, stored_hash: &str) -> Result<(), anyhow::Error> {
        let db_hash_parsed =
            PasswordHash::new(stored_hash).map_err(|_| anyhow!("Failed to parse stored hash"))?;
        Argon2::default()
            .verify_password(self.hash_part.as_bytes(), &db_hash_parsed)
            .map_err(|_| anyhow!("Unauthorized"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use common_domain::ids::ApiTokenId;
    use uuid::Uuid;

    use super::*;

    const KEY: &str = "pv_sand_5ldOh21Ipns1OpHzYbeAjvA87x3v/2vIOgNg2ElyLMxWAPn6Xz";

    fn id_of(validator: &ApiTokenValidator) -> ApiTokenId {
        ApiTokenId::from_const(validator.extract_identifier().unwrap())
    }

    #[test]
    fn test_parse_api_key() {
        let api_key = ApiTokenValidator::parse_api_key(KEY).unwrap();
        assert_eq!(api_key.id_part, "2vIOgNg2ElyLMxWAPn6Xz");

        assert_eq!(
            api_key.extract_identifier().unwrap(),
            Uuid::parse_str("018cb5a1-2ca6-7d0a-9090-319762bf129b").unwrap()
        );
    }

    #[test]
    fn fingerprint_is_stable_for_the_same_credential() {
        let a = ApiTokenValidator::parse_api_key(KEY).unwrap();
        let b = ApiTokenValidator::parse_api_key(KEY).unwrap();

        assert_eq!(
            a.credential_fingerprint(&id_of(&a)),
            b.credential_fingerprint(&id_of(&b))
        );
    }

    #[test]
    fn fingerprint_differs_when_only_the_secret_differs() {
        let real = ApiTokenValidator::parse_api_key(KEY).unwrap();
        let forged = ApiTokenValidator::parse_api_key(
            "pv_sand_0000000000000000000000000000/2vIOgNg2ElyLMxWAPn6Xz",
        )
        .unwrap();

        // Same token id, so an id-keyed cache would collide here.
        assert_eq!(id_of(&real), id_of(&forged));
        assert_ne!(
            real.credential_fingerprint(&id_of(&real)),
            forged.credential_fingerprint(&id_of(&forged))
        );
    }

    #[test]
    fn fingerprint_differs_when_only_the_id_differs() {
        let a = ApiTokenValidator::parse_api_key(KEY).unwrap();
        let other_id = ApiTokenId::from_const(Uuid::from_u128(1));

        assert_ne!(
            a.credential_fingerprint(&id_of(&a)),
            a.credential_fingerprint(&other_id)
        );
    }

    #[test]
    fn fingerprint_is_not_renderable() {
        let a = ApiTokenValidator::parse_api_key(KEY).unwrap();
        let rendered = format!("{:?}", a.credential_fingerprint(&id_of(&a)));

        assert_eq!(rendered, "CredentialFingerprint(redacted)");
    }
}
