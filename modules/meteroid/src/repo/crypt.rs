use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use error_stack::{Result, ResultExt};
use secrecy::{ExposeSecret, SecretString};

const NONCE_SIZE: usize = 12;

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

pub fn encrypt(crypt_key: &SecretString, value: &str) -> Result<SecretString, EncryptionError> {
    let cipher = ChaCha20Poly1305::new_from_slice(crypt_key.expose_secret().as_bytes())
        .change_context(EncryptionError::InvalidKey)?;

    let nonce = generate_nonce(crypt_key);

    let ciphertext: Vec<u8> = cipher
        .encrypt(nonce, value.as_bytes())
        .map_err(|_| EncryptionError::EncryptError)?;

    Ok(SecretString::new(hex::encode(ciphertext)))
}

pub fn decrypt(key: &SecretString, value: &str) -> Result<SecretString, EncryptionError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key.expose_secret().as_bytes())
        .change_context(EncryptionError::InvalidKey)?;

    let nonce = generate_nonce(key);

    let ciphertext = hex::decode(value).change_context(EncryptionError::InvalidHex)?;

    let decoded = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| EncryptionError::DecryptError)?;

    let plaintext = String::from_utf8(decoded).change_context(EncryptionError::DecryptError)?;

    Ok(SecretString::new(plaintext))
}

fn generate_nonce(key: &SecretString) -> &Nonce {
    Nonce::from_slice(key.expose_secret()[0..NONCE_SIZE].as_bytes())
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret, SecretString};

    #[test]
    fn test_encrypt_decrypt() {
        // (key, raw, encrypted)
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
            let key: SecretString = SecretString::new(key_str.into());

            let encrypted = super::encrypt(&key, raw_str).unwrap();

            assert_eq!(encrypted.expose_secret().as_str(), encrypted_str);

            let decrypted = super::decrypt(&key, encrypted.expose_secret().as_str()).unwrap();

            assert_eq!(decrypted.expose_secret().as_str(), raw_str);
        }
    }
}
