use anyhow::anyhow;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use sha2::{Digest, Sha256};

/// SHA-256 of the secret part of a presented API key.
///
/// Safe to use as a cache key: it is derived from the secret but does not expose it.
pub type ApiTokenSecretDigest = [u8; 32];

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

    /// The random segment, i.e. what is argon2-hashed and stored. Lets a caller-supplied
    /// key be installed verbatim instead of re-deriving the key format elsewhere.
    pub fn hash_part(&self) -> &str {
        &self.hash_part
    }

    pub fn extract_identifier(&self) -> Result<uuid::Uuid, anyhow::Error> {
        // Decode the identifier from base62 to UUID
        let id_u128 =
            base62::decode(&self.id_part).map_err(|_| anyhow!("Failed to decode identifier"))?;
        Ok(uuid::Uuid::from_u128(id_u128))
    }

    /// Digest of the presented secret, for use in validation caches.
    ///
    /// Callers must never key a validation cache on the token id alone: that would let any
    /// request quoting a known id reuse an entry populated by the legitimate secret.
    pub fn secret_digest(&self) -> ApiTokenSecretDigest {
        Sha256::digest(self.hash_part.as_bytes()).into()
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
    use uuid::Uuid;

    use super::*;

    #[test]
    fn test_parse_api_key() {
        let api_key = ApiTokenValidator::parse_api_key(
            "pv_sand_5ldOh21Ipns1OpHzYbeAjvA87x3v/2vIOgNg2ElyLMxWAPn6Xz",
        )
        .unwrap();
        assert_eq!(api_key.id_part, "2vIOgNg2ElyLMxWAPn6Xz");

        assert_eq!(
            api_key.extract_identifier().unwrap(),
            Uuid::parse_str("018cb5a1-2ca6-7d0a-9090-319762bf129b").unwrap()
        );
    }

    #[test]
    fn test_secret_digest_separates_keys_sharing_an_id() {
        let good = ApiTokenValidator::parse_api_key(
            "pv_sand_5ldOh21Ipns1OpHzYbeAjvA87x3v/2vIOgNg2ElyLMxWAPn6Xz",
        )
        .unwrap();
        let forged = ApiTokenValidator::parse_api_key(
            "pv_sand_0000000000000000000000000000000/2vIOgNg2ElyLMxWAPn6Xz",
        )
        .unwrap();

        assert_eq!(
            good.extract_identifier().unwrap(),
            forged.extract_identifier().unwrap()
        );
        assert_ne!(good.secret_digest(), forged.secret_digest());
    }

    #[test]
    fn test_secret_digest_is_stable_and_hides_the_secret() {
        let key = "pv_sand_5ldOh21Ipns1OpHzYbeAjvA87x3v/2vIOgNg2ElyLMxWAPn6Xz";
        let digest = ApiTokenValidator::parse_api_key(key)
            .unwrap()
            .secret_digest();

        assert_eq!(
            digest,
            ApiTokenValidator::parse_api_key(key)
                .unwrap()
                .secret_digest()
        );
        assert!(!digest.windows(4).any(|w| w == b"5ldO"));
    }
}
