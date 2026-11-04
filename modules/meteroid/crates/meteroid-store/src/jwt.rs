//! The only way to sign or verify JWTs (clippy bans `jsonwebtoken::{encode, decode}`): each call
//! installs an HS256 backend first, since jsonwebtoken panics without one.

use hmac::{Hmac, KeyInit, Mac};
use jsonwebtoken::crypto::{CryptoProvider, JwkUtils, JwtSigner, JwtVerifier};
use jsonwebtoken::errors::{ErrorKind, Result};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, TokenData, Validation};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::Sha256;

#[allow(clippy::disallowed_methods)]
pub fn encode<T: Serialize>(header: &Header, claims: &T, key: &EncodingKey) -> Result<String> {
    install();
    jsonwebtoken::encode(header, claims, key)
}

#[allow(clippy::disallowed_methods)]
pub fn decode<T: DeserializeOwned>(
    token: impl AsRef<[u8]>,
    key: &DecodingKey,
    validation: &Validation,
) -> Result<TokenData<T>> {
    install();
    jsonwebtoken::decode(token, key, validation)
}

fn install() {
    // Fails only when already installed.
    let _ = HS256.install_default();
}

static HS256: CryptoProvider = CryptoProvider {
    signer_factory: |alg, key: &EncodingKey| {
        Ok(Box::new(Hs256::new(alg, key.try_get_hmac_secret())?) as Box<dyn JwtSigner>)
    },
    verifier_factory: |alg, key: &DecodingKey| {
        Ok(Box::new(Hs256::new(alg, key.try_get_hmac_secret())?) as Box<dyn JwtVerifier>)
    },
    jwk_utils: JwkUtils::new_unimplemented(),
};

struct Hs256(Hmac<Sha256>);

impl Hs256 {
    fn new(algorithm: &Algorithm, secret: Result<&[u8]>) -> Result<Self> {
        if *algorithm != Algorithm::HS256 {
            return Err(ErrorKind::InvalidAlgorithm.into());
        }
        Hmac::new_from_slice(secret?)
            .map(Self)
            .map_err(|_| ErrorKind::InvalidKeyFormat.into())
    }
}

impl signature::Signer<Vec<u8>> for Hs256 {
    fn try_sign(&self, msg: &[u8]) -> std::result::Result<Vec<u8>, signature::Error> {
        Ok(self
            .0
            .clone()
            .chain_update(msg)
            .finalize()
            .into_bytes()
            .to_vec())
    }
}

impl signature::Verifier<Vec<u8>> for Hs256 {
    fn verify(&self, msg: &[u8], sig: &Vec<u8>) -> std::result::Result<(), signature::Error> {
        self.0
            .clone()
            .chain_update(msg)
            .verify_slice(sig)
            .map_err(|_| signature::Error::new())
    }
}

impl JwtSigner for Hs256 {
    fn algorithm(&self) -> Algorithm {
        Algorithm::HS256
    }
}

impl JwtVerifier for Hs256 {
    fn algorithm(&self) -> Algorithm {
        Algorithm::HS256
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn token(secret: &[u8], header: Header) -> Result<String> {
        let claims = json!({"sub": "x", "exp": 4_102_444_800u64});
        encode(&header, &claims, &EncodingKey::from_secret(secret))
    }

    #[test]
    fn round_trips_and_rejects_another_secret() {
        let token = token(b"secret", Header::default()).unwrap();
        let validation = Validation::default();

        let decoded = decode::<Value>(&token, &DecodingKey::from_secret(b"secret"), &validation);
        assert_eq!(decoded.unwrap().claims["sub"], "x");

        let other = decode::<Value>(&token, &DecodingKey::from_secret(b"other"), &validation);
        assert!(matches!(
            other.unwrap_err().kind(),
            ErrorKind::InvalidSignature
        ));
    }

    #[test]
    fn refuses_other_hmac_algorithms() {
        let err = token(b"secret", Header::new(Algorithm::HS512)).unwrap_err();
        assert!(matches!(err.kind(), ErrorKind::InvalidAlgorithm));
    }
}
