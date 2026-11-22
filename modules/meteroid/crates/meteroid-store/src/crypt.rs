use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, KeyInit},
};
use error_stack::{Report, ResultExt};
use secrecy::{ExposeSecret, SecretString};

const NONCE_SIZE: usize = 12;

/// Prefix of values sealed under a random nonce, stored as hex(nonce || ciphertext).
/// Unprefixed values are legacy ones, sealed under a nonce derived from the key.
const RANDOM_NONCE_PREFIX: &str = "v2:";

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum EncryptionError {
    #[error("Invalid key")]
    InvalidKey,
    #[error("Invalid hex")]
    InvalidHex,
    #[error("Encryption error")]
    EncryptError,
    #[error("Decryption error")]
    DecryptError,
}

pub fn encrypt(crypt_key: &SecretString, value: &str) -> Result<String, Report<EncryptionError>> {
    let nonce: [u8; NONCE_SIZE] = rand::random();
    let mut sealed = nonce.to_vec();
    sealed.extend(seal(crypt_key, &Nonce::from(nonce), value)?);

    Ok(format!("{RANDOM_NONCE_PREFIX}{}", hex::encode(sealed)))
}

/// Whether the value was sealed under the legacy key-derived nonce, and should be re-encrypted.
pub fn is_legacy(value: &str) -> bool {
    !value.starts_with(RANDOM_NONCE_PREFIX)
}

pub fn decrypt(key: &SecretString, value: &str) -> Result<SecretString, Report<EncryptionError>> {
    let (nonce, ciphertext) = match value.strip_prefix(RANDOM_NONCE_PREFIX) {
        Some(sealed) => {
            let bytes = hex::decode(sealed).change_context(EncryptionError::InvalidHex)?;
            if bytes.len() < NONCE_SIZE {
                return Err(Report::new(EncryptionError::DecryptError));
            }
            let (nonce, ciphertext) = bytes.split_at(NONCE_SIZE);
            (to_nonce(nonce)?, ciphertext.to_vec())
        }
        None => (
            legacy_nonce(key)?,
            hex::decode(value).change_context(EncryptionError::InvalidHex)?,
        ),
    };

    let decoded = cipher(key)?
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|_| EncryptionError::DecryptError)?;

    let plaintext = String::from_utf8(decoded).change_context(EncryptionError::DecryptError)?;

    Ok(SecretString::from(plaintext))
}

fn seal(
    key: &SecretString,
    nonce: &Nonce,
    value: &str,
) -> Result<Vec<u8>, Report<EncryptionError>> {
    cipher(key)?
        .encrypt(nonce, value.as_bytes())
        .map_err(|_| Report::new(EncryptionError::EncryptError))
}

fn cipher(key: &SecretString) -> Result<ChaCha20Poly1305, Report<EncryptionError>> {
    ChaCha20Poly1305::new_from_slice(key.expose_secret().as_bytes())
        .change_context(EncryptionError::InvalidKey)
}

fn legacy_nonce(key: &SecretString) -> Result<Nonce, Report<EncryptionError>> {
    let prefix = key
        .expose_secret()
        .as_bytes()
        .get(..NONCE_SIZE)
        .ok_or_else(|| Report::new(EncryptionError::InvalidKey))?;
    to_nonce(prefix)
}

fn to_nonce(bytes: &[u8]) -> Result<Nonce, Report<EncryptionError>> {
    let bytes: [u8; NONCE_SIZE] = bytes
        .try_into()
        .map_err(|_| Report::new(EncryptionError::DecryptError))?;
    Ok(Nonce::from(bytes))
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret, SecretString};

    const KEY: &str = "12345678901234567890123456789012";

    #[test]
    fn decrypts_legacy_values() {
        // (key, raw, sealed under the key-derived nonce)
        let test_data = [
            (
                "12345678901234567890123456789012",
                "RawValue",
                "d0bcdfc3a79f0bd426964fca333c19fb354fc6b22b60f121",
            ),
            (
                "12345678901234567890123456789012",
                "RawValueApiKey",
                "d0bcdfc3a79f0bd486619ed93435d2e2e1a4e533097cf323ed9667da08c5",
            ),
            (
                "023456F8901234G67890123456789019",
                "RawValue",
                "5bfaa24e1b3bcf556345fba291af65bf3d87c4cf638f81ec",
            ),
        ];

        for (key_str, raw_str, encrypted_str) in test_data {
            let key: SecretString = SecretString::from(key_str.to_owned());

            assert!(super::is_legacy(encrypted_str));
            let decrypted = super::decrypt(&key, encrypted_str).unwrap();

            assert_eq!(decrypted.expose_secret(), raw_str);
        }
    }

    #[test]
    fn encrypts_each_value_under_a_fresh_nonce() {
        let key = SecretString::from(KEY);

        let first = super::encrypt(&key, "RawValue").unwrap();
        let second = super::encrypt(&key, "RawValue").unwrap();

        assert_ne!(first, second);
        assert!(!super::is_legacy(&first));
        assert_eq!(
            super::decrypt(&key, &first).unwrap().expose_secret(),
            "RawValue"
        );
        assert_eq!(
            super::decrypt(&key, &second).unwrap().expose_secret(),
            "RawValue"
        );
    }

    fn sealed_with_random_nonce(key: &SecretString, raw: &str, nonce: [u8; 12]) -> String {
        let mut bytes = nonce.to_vec();
        bytes.extend(super::seal(key, &nonce.into(), raw).unwrap());
        format!("{}{}", super::RANDOM_NONCE_PREFIX, hex::encode(bytes))
    }

    #[test]
    fn decrypts_values_sealed_under_a_random_nonce() {
        let key = SecretString::from(KEY);
        let sealed = sealed_with_random_nonce(&key, "RawValue", *b"random nonce");

        let decrypted = super::decrypt(&key, &sealed).unwrap();

        assert_eq!(decrypted.expose_secret(), "RawValue");
    }

    #[test]
    fn rejects_a_tampered_random_nonce_value() {
        let key = SecretString::from(KEY);
        let mut sealed = sealed_with_random_nonce(&key, "RawValue", *b"random nonce");
        let last = sealed.pop().unwrap();
        sealed.push(if last == '0' { '1' } else { '0' });

        assert!(super::decrypt(&key, &sealed).is_err());
        assert!(super::decrypt(&key, "v2:00").is_err());
    }

    #[test]
    fn rejects_malformed_or_foreign_random_nonce_values() {
        let key = SecretString::from(KEY);
        let sealed = sealed_with_random_nonce(&key, "RawValue", *b"random nonce");
        let legacy = "d0bcdfc3a79f0bd426964fca333c19fb354fc6b22b60f121";
        let mut tampered_nonce = sealed.clone();
        tampered_nonce.replace_range(3..5, "00");
        let other_key = SecretString::from("023456F8901234G67890123456789019");

        for value in [
            "v2:".to_owned(),
            "v2:abc".to_owned(),
            format!("v2:{}", hex::encode(b"random nonce")),
            format!("v2:{legacy}"),
            sealed.replacen("v2:", "V2:", 1),
            tampered_nonce,
        ] {
            assert!(super::decrypt(&key, &value).is_err(), "{value}");
        }
        assert!(super::decrypt(&other_key, &sealed).is_err());
    }
}
