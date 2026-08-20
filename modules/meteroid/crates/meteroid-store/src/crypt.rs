use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use error_stack::{Report, ResultExt};
use secrecy::{ExposeSecret, SecretString};

const NONCE_SIZE: usize = 12;

/// Versioned envelope tag for ciphertext carrying its own random nonce.
///
/// Legacy ciphertext is bare hex, and `m`/`t`/`r` are not hex digits, so this prefix is an
/// unambiguous discriminator between the two formats.
pub const ENVELOPE_V1_PREFIX: &str = "mtr1:";
const ENVELOPE_V1: &str = "mtr1";

#[derive(Debug, thiserror::Error, PartialEq, Clone)]
pub enum EncryptionError {
    #[error("Invalid key")]
    InvalidKey,
    #[error("Invalid hex")]
    InvalidHex,
    #[error("Unsupported ciphertext envelope version")]
    UnsupportedVersion,
    #[error("Malformed ciphertext envelope")]
    MalformedEnvelope,
    #[error("Encryption error")]
    EncryptError,
    #[error("Decryption error")]
    DecryptError,
}

/// True for ciphertext written before versioned envelopes, i.e. still using the key-derived nonce.
pub fn is_legacy_envelope(value: &str) -> bool {
    !value.starts_with(ENVELOPE_V1_PREFIX)
}

pub fn encrypt(crypt_key: &SecretString, value: &str) -> Result<String, Report<EncryptionError>> {
    let cipher = cipher(crypt_key)?;

    // A fresh nonce per call. ChaCha20-Poly1305 loses both confidentiality and authenticity if a
    // nonce is ever reused under the same key, so this must never be derived from anything.
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, value.as_bytes())
        .map_err(|_| EncryptionError::EncryptError)?;

    Ok(format!(
        "{ENVELOPE_V1}:{}:{}",
        hex::encode(nonce),
        hex::encode(ciphertext)
    ))
}

pub fn decrypt(key: &SecretString, value: &str) -> Result<SecretString, Report<EncryptionError>> {
    let cipher = cipher(key)?;

    let Some((version, rest)) = value.split_once(':') else {
        return decrypt_legacy(&cipher, key, value);
    };

    if version != ENVELOPE_V1 {
        return Err(Report::new(EncryptionError::UnsupportedVersion));
    }

    let (nonce_hex, ciphertext_hex) = rest
        .split_once(':')
        .ok_or(EncryptionError::MalformedEnvelope)?;

    let nonce_bytes = hex::decode(nonce_hex).change_context(EncryptionError::InvalidHex)?;
    if nonce_bytes.len() != NONCE_SIZE {
        return Err(Report::new(EncryptionError::MalformedEnvelope));
    }

    let ciphertext = hex::decode(ciphertext_hex).change_context(EncryptionError::InvalidHex)?;

    open(&cipher, Nonce::from_slice(&nonce_bytes), &ciphertext)
}

/// Reads ciphertext written with the key-derived nonce.
///
/// Retained only so the startup migration can rewrite existing rows; nothing writes this format.
fn decrypt_legacy(
    cipher: &ChaCha20Poly1305,
    key: &SecretString,
    value: &str,
) -> Result<SecretString, Report<EncryptionError>> {
    let ciphertext = hex::decode(value).change_context(EncryptionError::InvalidHex)?;
    let nonce = legacy_nonce(key);

    open(cipher, nonce, &ciphertext)
}

fn open(
    cipher: &ChaCha20Poly1305,
    nonce: &Nonce,
    ciphertext: &[u8],
) -> Result<SecretString, Report<EncryptionError>> {
    let decoded = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| EncryptionError::DecryptError)?;

    let plaintext = String::from_utf8(decoded).change_context(EncryptionError::DecryptError)?;

    Ok(SecretString::from(plaintext))
}

fn cipher(key: &SecretString) -> Result<ChaCha20Poly1305, Report<EncryptionError>> {
    ChaCha20Poly1305::new_from_slice(key.expose_secret().as_bytes())
        .change_context(EncryptionError::InvalidKey)
}

fn legacy_nonce(key: &SecretString) -> &Nonce {
    // https://github.com/RustCrypto/AEADs/issues/730
    #[allow(deprecated)]
    Nonce::from_slice(&key.expose_secret().as_bytes()[0..NONCE_SIZE])
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::{ExposeSecret, SecretString};

    const KEY: &str = "12345678901234567890123456789012";

    fn key() -> SecretString {
        SecretString::from(KEY.to_owned())
    }

    #[test]
    fn round_trips() {
        let encrypted = encrypt(&key(), "RawValue").unwrap();

        assert!(encrypted.starts_with(ENVELOPE_V1_PREFIX));
        assert_eq!(
            decrypt(&key(), &encrypted).unwrap().expose_secret(),
            "RawValue"
        );
    }

    #[test]
    fn same_plaintext_never_produces_the_same_envelope() {
        let a = encrypt(&key(), "RawValue").unwrap();
        let b = encrypt(&key(), "RawValue").unwrap();

        assert_ne!(
            a, b,
            "nonce reuse: identical plaintext produced identical ciphertext"
        );
    }

    #[test]
    fn nonce_is_full_length_and_unique_across_calls() {
        let nonces: std::collections::HashSet<String> = (0..64)
            .map(|_| {
                let envelope = encrypt(&key(), "RawValue").unwrap();
                let nonce = envelope.split(':').nth(1).unwrap().to_string();
                assert_eq!(nonce.len(), NONCE_SIZE * 2);
                nonce
            })
            .collect();

        assert_eq!(nonces.len(), 64);
    }

    /// Known-answer vectors produced by the key-derived-nonce scheme, kept so the migration path
    /// stays readable. Nothing writes this format any more.
    #[test]
    fn legacy_ciphertext_still_decrypts() {
        let vectors = [
            (
                KEY,
                "RawValue",
                "d0bcdfc3a79f0bd426964fca333c19fb354fc6b22b60f121",
            ),
            (
                KEY,
                "RawValueApiKey",
                "d0bcdfc3a79f0bd486619ed93435d2e2e1a4e533097cf323ed9667da08c5",
            ),
            (
                "023456F8901234G67890123456789019",
                "RawValue",
                "5bfaa24e1b3bcf556345fba291af65bf3d87c4cf638f81ec",
            ),
        ];

        for (key_str, raw, encrypted) in vectors {
            let key = SecretString::from(key_str.to_owned());

            assert!(is_legacy_envelope(encrypted));
            assert_eq!(decrypt(&key, encrypted).unwrap().expose_secret(), raw);
        }
    }

    #[test]
    fn malformed_envelopes_fail_closed() {
        let valid = encrypt(&key(), "RawValue").unwrap();
        let (_, rest) = valid.split_once(':').unwrap();
        let (nonce_hex, ciphertext_hex) = rest.split_once(':').unwrap();

        let cases = [
            // unknown version
            format!("mtr2:{nonce_hex}:{ciphertext_hex}"),
            // version tag but no nonce/ciphertext separator
            format!("mtr1:{nonce_hex}{ciphertext_hex}"),
            // nonce too short
            format!("mtr1:{}:{ciphertext_hex}", &nonce_hex[..20]),
            // nonce too long
            format!("mtr1:{nonce_hex}00:{ciphertext_hex}"),
            // non-hex nonce
            format!("mtr1:{}:{ciphertext_hex}", "z".repeat(24)),
            // non-hex ciphertext
            format!("mtr1:{nonce_hex}:zz"),
            // truncated ciphertext defeats the authentication tag
            format!(
                "mtr1:{nonce_hex}:{}",
                &ciphertext_hex[..ciphertext_hex.len() - 2]
            ),
            // empty
            String::new(),
        ];

        for case in cases {
            assert!(
                decrypt(&key(), &case).is_err(),
                "malformed envelope was accepted: {case}"
            );
        }
    }

    #[test]
    fn tampered_ciphertext_fails_authentication() {
        let valid = encrypt(&key(), "RawValue").unwrap();
        let mut bytes: Vec<char> = valid.chars().collect();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == '0' { '1' } else { '0' };
        let tampered: String = bytes.into_iter().collect();

        assert_eq!(
            decrypt(&key(), &tampered).unwrap_err().current_context(),
            &EncryptionError::DecryptError
        );
    }

    #[test]
    fn wrong_key_fails_closed() {
        let encrypted = encrypt(&key(), "RawValue").unwrap();
        let other = SecretString::from("00000000000000000000000000000000".to_owned());

        assert!(decrypt(&other, &encrypted).is_err());
    }
}
