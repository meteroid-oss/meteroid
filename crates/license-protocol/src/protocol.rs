//! Wire protocol between a self-hosted instance and Meteroid's license server.
//!
//! Requests are signed with the instance key registered at activation, over the exact body
//! bytes, in [`SIGNATURE_HEADER`]. Each response rotates the refresh token.

use crate::error::LicenseError;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ACTIVATE_PATH: &str = "/licensing/v1/activate";
pub const REFRESH_PATH: &str = "/licensing/v1/refresh";
pub const CLAIM_PATH: &str = "/licensing/v1/claim";
pub const OFFERS_PATH: &str = "/licensing/v1/offers";
pub const JWKS_PATH: &str = "/licensing/v1/jwks";
pub const EVENTS_PATH: &str = "/licensing/v1/events";
pub const CONFIRM_PATH: &str = "/licensing/v1/confirm";
pub const REPORT_PATH: &str = "/licensing/v1/report";
/// Most events one [`UsageEventsRequest`] may carry.
pub const MAX_USAGE_EVENTS: usize = 100;
/// Most days one [`UsageReportRequest`] may carry.
pub const MAX_REPORT_DAYS: usize = 31;
/// The event a [`DailyReport`] is ingested as.
pub const SELF_HOSTED_STATS_CODE: &str = "self_hosted_stats";
pub const SIGNATURE_HEADER: &str = "x-meteroid-signature";
/// How far a request's `sent_at` may drift from the server clock.
pub const MAX_CLOCK_SKEW_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateRequest {
    pub activation_code: String,
    pub instance_id: String,
    pub instance_public_key: String,
    pub sent_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub instance_id: String,
    pub refresh_token: String,
    pub sent_at: i64,
    /// Random per process: tells a retry of the holder from another copy of the installation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Shown to the issuer when another copy of the installation holds the license.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    /// Takes the license over from another copy of this installation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub take_over: bool,
}

/// Sent once the tokens of a refresh, activation or claim are stored. Until then, the token
/// they replaced is accepted from any process: the response may have been lost to a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmRequest {
    pub instance_id: String,
    pub refresh_token: String,
    pub sent_at: i64,
    pub session: String,
}

/// The 409 refusing a copy of an installation while another copy keeps refreshing. Not final:
/// the copy takes over once the holder stops, or with [`RefreshRequest::take_over`].
pub const HELD_BY_ANOTHER_COPY: &str = "another copy of this installation holds this license";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseResponse {
    /// The signed license JWT.
    pub license: String,
    /// Replaces the refresh token used for this call.
    pub refresh_token: String,
}

/// The body of a 403 that revokes a license at once. A bare 403 only stops its renewal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevocationNotice {
    pub revoked: String,
    pub message: String,
}

impl RevocationNotice {
    pub const IMMEDIATE: &'static str = "immediately";

    pub fn immediate(message: String) -> Self {
        Self {
            revoked: Self::IMMEDIATE.to_string(),
            message,
        }
    }

    /// Whether a rejection body is this notice.
    pub fn is_immediate(body: &str) -> bool {
        serde_json::from_str::<Self>(body).is_ok_and(|n| n.revoked == Self::IMMEDIATE)
    }
}

/// Usage events an installation reports with its license credentials. The server attributes
/// them to the license's customer, whatever the installation claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEventsRequest {
    pub instance_id: String,
    pub refresh_token: String,
    pub sent_at: i64,
    pub events: Vec<UsageEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageEvent {
    /// Deduplicates the event: sending it again is not counted twice.
    pub event_id: String,
    pub code: String,
    /// RFC 3339.
    pub timestamp: String,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

/// What an installation of Meteroid reports about each closed UTC day. The server turns it into
/// one [`SELF_HOSTED_STATS_CODE`] event of the license's customer per day.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageReportRequest {
    pub instance_id: String,
    pub refresh_token: String,
    pub sent_at: i64,
    pub days: Vec<DailyReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyReport {
    pub day: chrono::NaiveDate,
    /// Pre-tax revenue finalized that day by production tenants, per currency.
    #[serde(default)]
    pub revenue: Vec<CurrencyRevenue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrencyRevenue {
    pub currency: String,
    /// Invoices minus credit notes, in minor units.
    pub amount_cents: i64,
    pub invoice_count: u32,
    pub credit_note_count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageReportResponse {}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageEventsResponse {
    pub failures: Vec<UsageEventFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEventFailure {
    pub event_id: String,
    pub reason: String,
}

/// Last line of a usage file signed by its installation; the signature covers every byte
/// before it, and CSV importers skip it as a comment.
pub const USAGE_SIGNATURE_PREFIX: &str = "# meteroid-usage-signature v1";

pub fn sign_usage_file(key: &InstanceKey, instance_id: &str, content: &str) -> String {
    let mut content = content.to_string();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    let signature = key.sign(content.as_bytes());
    format!(
        "{content}{USAGE_SIGNATURE_PREFIX} instance={instance_id} key={} sig={signature}\n",
        key.public_key()
    )
}

pub struct SignedUsageFile {
    pub instance_id: String,
    /// The key that signed the file: compare it with the one the installation shows.
    pub public_key: String,
}

/// Checks a usage file's signature against the key it names, and that key against
/// `expected_key` when given.
pub fn verify_usage_file(
    file: &str,
    expected_key: Option<&str>,
) -> Result<SignedUsageFile, LicenseError> {
    let trimmed = file.trim_end_matches(['\n', '\r']);
    let start = trimmed.rfind('\n').map_or(0, |i| i + 1);
    let line = trimmed[start..]
        .strip_prefix(USAGE_SIGNATURE_PREFIX)
        .ok_or_else(|| LicenseError::DecodeError("the file is not signed".to_string()))?;
    let field = |name: &str| {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(name)?.strip_prefix('='))
            .ok_or_else(|| LicenseError::DecodeError(format!("signature line misses {name}")))
    };
    let public_key = field("key")?;
    if expected_key.is_some_and(|expected| expected != public_key) {
        return Err(LicenseError::InvalidInstanceSignature);
    }
    verify_instance_signature(public_key, &file.as_bytes()[..start], field("sig")?)?;
    Ok(SignedUsageFile {
        instance_id: field("instance")?.to_string(),
        public_key: public_key.to_string(),
    })
}

/// Picks up a license bought for this instance from a purchase link. Signed like refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimRequest {
    pub instance_id: String,
    pub sent_at: i64,
}

/// A way to subscribe offered to installations (a listed purchase link).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub plan_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The purchase page; append `?license_request=` with a signed [`PurchaseRequest`].
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OffersResponse {
    pub offers: Vec<Offer>,
}

/// Lifetime of a signed [`PurchaseRequest`].
pub const PURCHASE_REQUEST_TTL_SECS: i64 = 24 * 3600;

/// Proves a purchase was started by the installation holding `instance_public_key`, so the
/// license of the resulting subscription is bound to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PurchaseRequest {
    pub instance_id: String,
    pub instance_public_key: String,
    pub issued_at: i64,
}

impl PurchaseRequest {
    /// `base64url(json).signature`, safe in a query string.
    pub fn sign(&self, key: &InstanceKey) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("serializes"));
        let signature = key.sign(payload.as_bytes());
        format!("{payload}.{signature}")
    }

    pub fn verify(token: &str, now: i64) -> Result<Self, LicenseError> {
        let (payload, signature) = token
            .split_once('.')
            .ok_or(LicenseError::InvalidInstanceSignature)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|e| LicenseError::DecodeError(e.to_string()))?;
        let request: Self =
            serde_json::from_slice(&bytes).map_err(|e| LicenseError::DecodeError(e.to_string()))?;
        verify_instance_signature(&request.instance_public_key, payload.as_bytes(), signature)?;
        let age = now - request.issued_at;
        if !(-MAX_CLOCK_SKEW_SECS..=PURCHASE_REQUEST_TTL_SECS).contains(&age) {
            return Err(LicenseError::Expired);
        }
        Ok(request)
    }
}

/// Path of a tenant's licensing endpoint, under its opaque slug; the unscoped paths address
/// Meteroid itself.
pub fn tenant_path(slug: &str, path: &str) -> String {
    let suffix = path.trim_start_matches("/licensing/v1");
    format!("/licensing/v1/t/{slug}{suffix}")
}

/// The instance's own Ed25519 key, generated once and kept in its database.
pub struct InstanceKey(SigningKey);

impl InstanceKey {
    pub fn from_bytes(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0.verifying_key().as_bytes())
    }

    pub fn sign(&self, payload: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(self.0.sign(payload).to_bytes())
    }
}

pub fn verify_instance_signature(
    public_key: &str,
    payload: &[u8],
    signature: &str,
) -> Result<(), LicenseError> {
    let decode = |s: &str| {
        URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| LicenseError::InvalidInstanceSignature)
    };
    let key_bytes: [u8; 32] = decode(public_key)?
        .try_into()
        .map_err(|_| LicenseError::InvalidInstanceSignature)?;
    let key =
        VerifyingKey::from_bytes(&key_bytes).map_err(|_| LicenseError::InvalidInstanceSignature)?;
    let signature = Signature::try_from(decode(signature)?.as_slice())
        .map_err(|_| LicenseError::InvalidInstanceSignature)?;
    key.verify(payload, &signature)
        .map_err(|_| LicenseError::InvalidInstanceSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_usage_file_verifies_against_the_instance_key_only() {
        let key = InstanceKey::from_bytes(&[1; 32]);
        let csv = "event_code,customer_id,event_id,amount\nrevenue,cus,inv_1,12000";
        let file = sign_usage_file(&key, "inst", csv);
        assert!(file.starts_with(csv));
        let signed = verify_usage_file(&file, None).unwrap();
        assert_eq!(signed.instance_id, "inst");
        assert_eq!(signed.public_key, key.public_key());
        assert!(verify_usage_file(&file, Some(&key.public_key())).is_ok());

        let other = InstanceKey::from_bytes(&[2; 32]);
        assert!(verify_usage_file(&file, Some(&other.public_key())).is_err());
        let resigned = file.replace(&key.public_key(), &other.public_key());
        assert!(verify_usage_file(&resigned, None).is_err());
        assert!(verify_usage_file(&file.replace("12000", "1"), None).is_err());
        assert!(verify_usage_file(csv, None).is_err());
    }

    #[test]
    fn a_purchase_request_is_bound_to_its_key_and_expires() {
        let key = InstanceKey::from_bytes(&[4; 32]);
        let request = PurchaseRequest {
            instance_id: "inst".to_string(),
            instance_public_key: key.public_key(),
            issued_at: 1_000,
        };
        let token = request.sign(&key);
        assert_eq!(PurchaseRequest::verify(&token, 1_100).unwrap(), request);
        assert!(PurchaseRequest::verify(&token, 1_000 + PURCHASE_REQUEST_TTL_SECS + 1).is_err());

        let other = InstanceKey::from_bytes(&[5; 32]);
        let forged = PurchaseRequest {
            instance_public_key: other.public_key(),
            ..request
        };
        let (payload, _) = forged
            .sign(&other)
            .split_once('.')
            .map(|(p, s)| (p.to_string(), s.to_string()))
            .unwrap();
        let (_, signature) = token.split_once('.').unwrap();
        assert!(PurchaseRequest::verify(&format!("{payload}.{signature}"), 1_100).is_err());
    }

    #[test]
    fn tenant_paths_extend_the_unscoped_ones() {
        assert_eq!(
            tenant_path("ten_1", CLAIM_PATH),
            "/licensing/v1/t/ten_1/claim"
        );
    }

    #[test]
    fn request_signatures_cover_the_exact_body() {
        let key = InstanceKey::from_bytes(&[3; 32]);
        let body = br#"{"instance_id":"a"}"#;
        let sig = key.sign(body);
        assert!(verify_instance_signature(&key.public_key(), body, &sig).is_ok());
        assert!(verify_instance_signature(&key.public_key(), b"{}", &sig).is_err());
        assert!(verify_instance_signature("garbage", body, &sig).is_err());
    }
}
