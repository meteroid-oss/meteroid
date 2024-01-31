use anyhow::anyhow;
use argon2::{Argon2, PasswordHash, PasswordVerifier};

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
            .rsplitn(2, '_')
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

    pub fn validate_hash(&self, stored_hash: &str) -> Result<(), anyhow::Error> {
        let db_hash_parsed =
            PasswordHash::new(stored_hash).map_err(|_| anyhow!("Failed to parse stored hash"))?;
        Argon2::default()
            .verify_password(&self.hash_part.as_bytes(), &db_hash_parsed)
            .map_err(|_| anyhow!("Unauthorized"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
}
