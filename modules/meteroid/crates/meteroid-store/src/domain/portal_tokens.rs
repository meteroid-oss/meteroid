use crate::StoreResult;
use crate::errors::StoreError;
use crate::jwt_claims::{PortalScope, ResourceAccess};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use common_domain::ids::{BaseId, PortalTokenId, TenantId};
use diesel_models::enums::PortalResourceKindEnum;
use diesel_models::portal_tokens::PortalTokenRow;
use hmac::{Hmac, KeyInit, Mac};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The portal tells an expired link (recoverable) from an invalid one by this message.
pub const PORTAL_LINK_EXPIRED: &str = "Portal link expired";
const DERIVATION_LABEL: &[u8] = b"meteroid.portal-token.v1:";
const TOKEN_LEN: usize = 32;

/// A link handed out (minted or reused) stays usable at least this long.
pub fn min_link_validity() -> Duration {
    Duration::days(10)
}

/// Told apart from legacy JWTs by shape: those are longer and carry dots.
pub fn is_portal_token(token: &str) -> bool {
    token.len() == TOKEN_LEN
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The secret is re-derivable from the row id, so a live link can be handed out again
/// without storing it. After a key rotation old links still resolve (by stored hash) but
/// stop being reused.
pub fn derive_portal_token(key: &SecretString, id: PortalTokenId) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.expose_secret().as_bytes()).expect("any key length");
    mac.update(DERIVATION_LABEL);
    mac.update(id.as_uuid().as_bytes());
    let digest = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(&digest[..TOKEN_LEN * 3 / 4])
}

pub fn hash_portal_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// `Manage` implies `Read`, so a scope set is either read-only or full.
pub fn canonical_scopes(scopes: Option<&[PortalScope]>) -> Vec<PortalScope> {
    match scopes {
        Some(scopes) if !scopes.contains(&PortalScope::Manage) => vec![PortalScope::Read],
        _ => PortalScope::ALL.to_vec(),
    }
}

fn scope_name(scope: PortalScope) -> &'static str {
    match scope {
        PortalScope::Read => "read",
        PortalScope::Manage => "manage",
    }
}

pub fn scope_names(scopes: &[PortalScope]) -> Vec<Option<String>> {
    scopes
        .iter()
        .map(|s| Some(scope_name(*s).to_string()))
        .collect()
}

fn parse_scopes(names: &[Option<String>]) -> StoreResult<Vec<PortalScope>> {
    names
        .iter()
        .map(|name| match name.as_deref() {
            Some("read") => Ok(PortalScope::Read),
            Some("manage") => Ok(PortalScope::Manage),
            other => {
                Err(StoreError::InvalidArgument(format!("unknown portal scope {other:?}")).into())
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct IssuedPortalToken {
    pub token: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalTokenStatus {
    Active,
    Expired,
    Revoked,
}

#[derive(Debug, Clone)]
pub struct PortalToken {
    pub id: PortalTokenId,
    pub tenant_id: TenantId,
    pub resource: ResourceAccess,
    /// Set on customer tokens only.
    pub scopes: Option<Vec<PortalScope>>,
    /// Minted with delegated or express credentials.
    pub delegated: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl PortalToken {
    pub fn status(&self, now: DateTime<Utc>) -> PortalTokenStatus {
        if self.revoked_at.is_some() {
            PortalTokenStatus::Revoked
        } else if self.expires_at.is_some_and(|exp| exp <= now) {
            PortalTokenStatus::Expired
        } else {
            PortalTokenStatus::Active
        }
    }
}

impl TryFrom<PortalTokenRow> for PortalToken {
    type Error = error_stack::Report<StoreError>;

    fn try_from(row: PortalTokenRow) -> StoreResult<Self> {
        let id = row.resource_id;
        let scopes = (row.resource_kind == PortalResourceKindEnum::Customer)
            .then(|| parse_scopes(&row.scopes))
            .transpose()?;
        let resource = match row.resource_kind {
            PortalResourceKindEnum::Invoice => ResourceAccess::Invoice(id.into()),
            PortalResourceKindEnum::Customer => ResourceAccess::Customer(id.into()),
            PortalResourceKindEnum::CheckoutSession => ResourceAccess::CheckoutSession(id.into()),
            PortalResourceKindEnum::Dashboard => ResourceAccess::Dashboard(id.into()),
            PortalResourceKindEnum::Quote => ResourceAccess::Quote {
                quote_id: id.into(),
                recipient_email: row.recipient_email.ok_or_else(|| {
                    StoreError::InvalidArgument("quote portal token without recipient".into())
                })?,
            },
        };
        Ok(Self {
            id: row.id,
            tenant_id: row.tenant_id,
            resource,
            scopes,
            delegated: row.delegated,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            last_used_at: row.last_used_at,
        })
    }
}

pub struct ResourceKey {
    pub kind: PortalResourceKindEnum,
    pub id: Uuid,
    pub recipient_email: Option<String>,
}

impl From<&ResourceAccess> for ResourceKey {
    fn from(resource: &ResourceAccess) -> Self {
        let (kind, id, recipient_email) = match resource {
            ResourceAccess::Invoice(id) => (PortalResourceKindEnum::Invoice, id.as_uuid(), None),
            ResourceAccess::Customer(id) => (PortalResourceKindEnum::Customer, id.as_uuid(), None),
            ResourceAccess::CheckoutSession(id) => {
                (PortalResourceKindEnum::CheckoutSession, id.as_uuid(), None)
            }
            ResourceAccess::Dashboard(id) => {
                (PortalResourceKindEnum::Dashboard, id.as_uuid(), None)
            }
            ResourceAccess::Quote {
                quote_id,
                recipient_email,
            } => (
                PortalResourceKindEnum::Quote,
                quote_id.as_uuid(),
                Some(recipient_email.clone()),
            ),
        };
        Self {
            kind,
            id,
            recipient_email,
        }
    }
}

/// Due date (else finalization) + 30 days, never more than 120 days past finalization.
pub fn invoice_link_expiry(
    finalized_at: Option<NaiveDateTime>,
    due_at: Option<NaiveDateTime>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let issued = finalized_at.map(|d| d.and_utc()).unwrap_or(now);
    let scheduled = (due_at.map(|d| d.and_utc()).unwrap_or(issued) + Duration::days(30))
        .min(issued + Duration::days(120));
    scheduled.max(now + min_link_validity())
}

/// Quote expiry + 30 days so a late reader still sees its outcome; 90 days without one.
pub fn quote_link_expiry(
    quote_expires_at: Option<NaiveDateTime>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    quote_expires_at
        .map(|d| d.and_utc() + Duration::days(30))
        .unwrap_or(now + Duration::days(90))
        .max(now + min_link_validity())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common_domain::ids::{InvoiceId, QuoteId};

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn derived_tokens_are_stable_per_id_and_key() {
        let key = SecretString::from("secret-a");
        let id = PortalTokenId::new();
        let token = derive_portal_token(&key, id);

        assert!(is_portal_token(&token));
        assert_eq!(token.len(), 32);
        assert_eq!(token, derive_portal_token(&key, id));
        assert_ne!(token, derive_portal_token(&key, PortalTokenId::new()));
        assert_ne!(
            token,
            derive_portal_token(&SecretString::from("secret-b"), id)
        );
        assert_ne!(hash_portal_token(&token), token);
    }

    #[test]
    fn jwts_are_not_portal_tokens() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJlLXNpZ25hdHVyZQ";
        assert!(!is_portal_token(jwt));
        assert!(!is_portal_token("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0"));
        assert!(!is_portal_token("not-a-token"));
        assert!(is_portal_token("abcdefghijklmnopqrstuvwxyz012-_9"));
    }

    #[test]
    fn scopes_collapse_to_read_only_or_full() {
        let full = PortalScope::ALL.to_vec();
        assert_eq!(canonical_scopes(None), full);
        assert_eq!(canonical_scopes(Some(&[PortalScope::Manage])), full);
        assert_eq!(
            canonical_scopes(Some(&[PortalScope::Read, PortalScope::Read])),
            vec![PortalScope::Read]
        );
        assert_eq!(parse_scopes(&scope_names(&full)).unwrap(), full);
        assert!(parse_scopes(&[Some("admin".into())]).is_err());
    }

    #[test]
    fn invoice_link_lives_30_days_past_due() {
        let now = at(2026, 1, 1);
        let finalized = Some(at(2026, 1, 1).naive_utc());
        let due = Some(at(2026, 1, 31).naive_utc());
        assert_eq!(invoice_link_expiry(finalized, due, now), at(2026, 3, 2));
    }

    #[test]
    fn invoice_link_without_due_date_counts_from_finalization() {
        let now = at(2026, 1, 1);
        let finalized = Some(at(2026, 1, 1).naive_utc());
        assert_eq!(invoice_link_expiry(finalized, None, now), at(2026, 1, 31));
    }

    #[test]
    fn invoice_link_is_capped_at_120_days_past_finalization() {
        let now = at(2026, 1, 1);
        let finalized = Some(at(2026, 1, 1).naive_utc());
        let due = Some(at(2026, 12, 31).naive_utc());
        assert_eq!(invoice_link_expiry(finalized, due, now), at(2026, 5, 1));
    }

    #[test]
    fn late_handout_still_gets_minimum_validity() {
        let now = at(2027, 1, 1);
        let finalized = Some(at(2026, 1, 1).naive_utc());
        let due = Some(at(2026, 1, 31).naive_utc());
        assert_eq!(invoice_link_expiry(finalized, due, now), at(2027, 1, 11));
        assert_eq!(quote_link_expiry(due, now), at(2027, 1, 11));
    }

    #[test]
    fn quote_link_follows_quote_expiry() {
        let now = at(2026, 1, 1);
        assert_eq!(
            quote_link_expiry(Some(at(2026, 2, 1).naive_utc()), now),
            at(2026, 3, 3)
        );
        assert_eq!(quote_link_expiry(None, now), at(2026, 4, 1));
    }

    #[test]
    fn status_prefers_revoked_over_expired() {
        let now = at(2026, 6, 1);
        let mut token = PortalToken {
            id: PortalTokenId::new(),
            tenant_id: TenantId::new(),
            resource: ResourceAccess::Invoice(InvoiceId::new()),
            scopes: None,
            delegated: false,
            expires_at: Some(at(2026, 7, 1)),
            revoked_at: None,
            last_used_at: None,
        };
        assert_eq!(token.status(now), PortalTokenStatus::Active);
        token.expires_at = Some(now);
        assert_eq!(token.status(now), PortalTokenStatus::Expired);
        token.revoked_at = Some(now);
        assert_eq!(token.status(now), PortalTokenStatus::Revoked);
    }

    #[test]
    fn quote_resource_key_keeps_recipient() {
        let quote_id = QuoteId::new();
        let key = ResourceKey::from(&ResourceAccess::Quote {
            quote_id,
            recipient_email: "a@b.co".into(),
        });
        assert_eq!(key.kind, PortalResourceKindEnum::Quote);
        assert_eq!(key.id, quote_id.as_uuid());
        assert_eq!(key.recipient_email.as_deref(), Some("a@b.co"));
    }
}
