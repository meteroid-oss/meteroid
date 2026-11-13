//! Self-hosted proof of work in the ALTCHA format: the server signs a challenge, the
//! browser finds the number behind it, and the solution travels back for a stateless check.
//! Challenges are bound to a scope (the purchase link token); replays are refused by the
//! caller, which records each spent challenge until it expires.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ALGORITHM: &str = "SHA-256";
/// Upper bound of the secret number: about 100k hashes on average, around a second in a browser.
pub const MAX_NUMBER: u64 = 200_000;
pub const CHALLENGE_TTL_SECS: i64 = 600;
const CLOCK_SKEW_SECS: i64 = 60;
const SALT_HEX_LENGTH: usize = 24;
const EXPIRES_DIGITS: usize = 10;
const MAX_SOLUTION_LENGTH: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub algorithm: String,
    pub challenge: String,
    pub maxnumber: u64,
    pub salt: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Solution {
    pub algorithm: String,
    pub challenge: String,
    pub number: u64,
    pub salt: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPow {
    pub challenge: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PowError {
    #[error("malformed proof of work")]
    Malformed,
    #[error("proof of work expired")]
    Expired,
    #[error("invalid proof of work")]
    Invalid,
}

#[derive(Clone)]
pub struct ProofOfWork {
    key: [u8; 32],
}

impl ProofOfWork {
    pub fn new(secret: &SecretString) -> Self {
        Self {
            key: hmac_sha256::HMAC::mac(
                b"meteroid.purchase.pow",
                secret.expose_secret().as_bytes(),
            ),
        }
    }

    /// A challenge only `scope` accepts.
    pub fn create_challenge(&self, scope: &str) -> Challenge {
        use rand::RngExt;
        let salt_bytes: [u8; SALT_HEX_LENGTH / 2] = rand::random();
        let expires = common_domain::clock::wall().timestamp() + CHALLENGE_TTL_SECS;
        let salt = format!("{}?expires={expires}", hex::encode(salt_bytes));
        let number = rand::rng().random_range(0..=MAX_NUMBER);
        let challenge = hash(&salt, number);
        Challenge {
            algorithm: ALGORITHM.to_string(),
            signature: self.sign(scope, &challenge),
            challenge,
            maxnumber: MAX_NUMBER,
            salt,
        }
    }

    /// Checks a base64(JSON) solution against `scope`.
    pub fn verify(&self, payload: &str, scope: &str) -> Result<VerifiedPow, PowError> {
        if payload.len() > MAX_SOLUTION_LENGTH {
            return Err(PowError::Malformed);
        }
        let bytes = STANDARD
            .decode(payload.trim())
            .map_err(|_| PowError::Malformed)?;
        let solution: Solution = serde_json::from_slice(&bytes).map_err(|_| PowError::Malformed)?;

        // The salt's shape is fixed so a digit can't move between the salt and the number.
        let expires = parse_salt(&solution.salt).ok_or(PowError::Invalid)?;
        if solution.algorithm != ALGORITHM || solution.number > MAX_NUMBER {
            return Err(PowError::Invalid);
        }
        if !constant_time_eq(
            self.sign(scope, &solution.challenge).as_bytes(),
            solution.signature.as_bytes(),
        ) {
            return Err(PowError::Invalid);
        }
        if !constant_time_eq(
            hash(&solution.salt, solution.number).as_bytes(),
            solution.challenge.as_bytes(),
        ) {
            return Err(PowError::Invalid);
        }

        let now = common_domain::clock::wall().timestamp();
        if expires < now {
            return Err(PowError::Expired);
        }
        if expires > now + CHALLENGE_TTL_SECS + CLOCK_SKEW_SECS {
            return Err(PowError::Invalid);
        }
        Ok(VerifiedPow {
            challenge: solution.challenge,
            expires_at: DateTime::from_timestamp(expires, 0).ok_or(PowError::Invalid)?,
        })
    }

    fn sign(&self, scope: &str, challenge: &str) -> String {
        let message = format!("{}:{scope}:{challenge}", scope.len());
        hex::encode(hmac_sha256::HMAC::mac(message.as_bytes(), self.key))
    }
}

pub fn hash(salt: &str, number: u64) -> String {
    hex::encode(Sha256::digest(format!("{salt}{number}").as_bytes()))
}

/// Brute-forces a challenge the way the browser widget does; for tests and tooling.
pub fn solve(challenge: &Challenge) -> Option<String> {
    let number =
        (0..=challenge.maxnumber).find(|n| hash(&challenge.salt, *n) == challenge.challenge)?;
    let solution = Solution {
        algorithm: challenge.algorithm.clone(),
        challenge: challenge.challenge.clone(),
        number,
        salt: challenge.salt.clone(),
        signature: challenge.signature.clone(),
    };
    Some(STANDARD.encode(serde_json::to_vec(&solution).ok()?))
}

/// `<24 lowercase hex>?expires=<10 digits>`, nothing else; returns the expiry.
fn parse_salt(salt: &str) -> Option<i64> {
    let (random, expires) = salt.split_once("?expires=")?;
    let lower_hex = |s: &str| s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if random.len() != SALT_HEX_LENGTH
        || !lower_hex(random)
        || expires.len() != EXPIRES_DIGITS
        || !expires.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    expires.parse().ok()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "link-token";

    fn pow() -> ProofOfWork {
        ProofOfWork::new(&SecretString::from("secret".to_string()))
    }

    fn encode(solution: &Solution) -> String {
        STANDARD.encode(serde_json::to_vec(solution).unwrap())
    }

    fn signed(pow: &ProofOfWork, salt: &str, number: u64) -> Solution {
        let challenge = hash(salt, number);
        Solution {
            algorithm: ALGORITHM.to_string(),
            signature: pow.sign(SCOPE, &challenge),
            challenge,
            number,
            salt: salt.to_string(),
        }
    }

    #[test]
    fn a_solved_challenge_verifies_for_its_scope_only() {
        let pow = pow();
        let challenge = pow.create_challenge(SCOPE);
        let solution = solve(&challenge).expect("solvable");
        let verified = pow.verify(&solution, SCOPE).unwrap();
        assert_eq!(verified.challenge, challenge.challenge);
        assert_eq!(
            pow.verify(&solution, "another-link"),
            Err(PowError::Invalid)
        );
    }

    #[test]
    fn forged_or_foreign_solutions_are_refused() {
        let pow = pow();
        let challenge = pow.create_challenge(SCOPE);
        let solution: Solution =
            serde_json::from_slice(&STANDARD.decode(solve(&challenge).unwrap()).unwrap()).unwrap();

        let wrong_number = Solution {
            number: (solution.number + 1) % (MAX_NUMBER + 1),
            ..solution.clone()
        };
        assert_eq!(
            pow.verify(&encode(&wrong_number), SCOPE),
            Err(PowError::Invalid)
        );

        let other_server = ProofOfWork::new(&SecretString::from("other".to_string()));
        assert_eq!(
            other_server.verify(&encode(&solution), SCOPE),
            Err(PowError::Invalid)
        );
        assert_eq!(pow.verify("not base64!", SCOPE), Err(PowError::Malformed));
    }

    /// Moving the number's first digit into the salt keeps sha256(salt + number) intact; the
    /// strict salt shape refuses it, so a solution can't be given a far-future expiry.
    #[test]
    fn splicing_digits_into_the_salt_is_refused() {
        let pow = pow();
        let expires = common_domain::clock::wall().timestamp() + 300;
        let salt = format!("{}?expires={expires}", "a".repeat(SALT_HEX_LENGTH));
        let genuine = signed(&pow, &salt, 12_345);
        assert!(pow.verify(&encode(&genuine), SCOPE).is_ok());

        let spliced = Solution {
            salt: format!("{salt}1"),
            number: 2_345,
            ..genuine
        };
        assert_eq!(spliced.challenge, hash(&spliced.salt, spliced.number));
        assert_eq!(pow.verify(&encode(&spliced), SCOPE), Err(PowError::Invalid));
    }

    #[test]
    fn expiry_must_be_close_and_in_the_future() {
        let pow = pow();
        let random = "b".repeat(SALT_HEX_LENGTH);
        let past = format!(
            "{random}?expires={}",
            common_domain::clock::wall().timestamp() - 1
        );
        assert_eq!(
            pow.verify(&encode(&signed(&pow, &past, 7)), SCOPE),
            Err(PowError::Expired)
        );
        let far = format!(
            "{random}?expires={}",
            common_domain::clock::wall().timestamp() + 86_400
        );
        assert_eq!(
            pow.verify(&encode(&signed(&pow, &far, 7)), SCOPE),
            Err(PowError::Invalid)
        );
        let upper = format!(
            "{}?expires={}",
            "B".repeat(SALT_HEX_LENGTH),
            common_domain::clock::wall().timestamp() + 60
        );
        assert_eq!(
            pow.verify(&encode(&signed(&pow, &upper, 7)), SCOPE),
            Err(PowError::Invalid)
        );
        let over = format!(
            "{random}?expires={}",
            common_domain::clock::wall().timestamp() + 60
        );
        assert_eq!(
            pow.verify(&encode(&signed(&pow, &over, MAX_NUMBER + 1)), SCOPE),
            Err(PowError::Invalid)
        );
    }
}
