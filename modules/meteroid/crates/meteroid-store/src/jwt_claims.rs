use crate::StoreResult;
use crate::errors::StoreError;
pub use common_domain::auth::PortalScope;
use common_domain::ids::{
    CheckoutSessionId, CustomerId, InvoiceId, QuoteId, SavedViewId, TenantId,
};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{DecodingKey, EncodingKey};
use secrecy::{ExposeSecret, SecretString};
use serde_with::skip_serializing_none;
// todo reuse in common-grpc as well

/// Keeps portal tokens apart from session JWTs, which share `jwt_secret`.
pub const PORTAL_TOKEN_AUDIENCE: &str = "meteroid:portal";

#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub enum ResourceAccess {
    CheckoutSession(CheckoutSessionId),
    Customer(CustomerId),
    Invoice(InvoiceId),
    Quote {
        quote_id: QuoteId,
        recipient_email: String,
    },
    Dashboard(SavedViewId),
}

/// Default lifetime for a customer portal token when the caller doesn't specify one.
pub fn default_customer_portal_token_ttl() -> chrono::Duration {
    chrono::Duration::hours(24)
}

#[skip_serializing_none]
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct PortalJwtClaims {
    iat: usize,
    pub exp: Option<usize>,
    /// Absent on tokens minted before it was introduced.
    pub aud: Option<String>,
    pub tenant_id: TenantId,
    pub resource: ResourceAccess,
    /// Customer tokens only. Absent on legacy customer tokens, which keep full access.
    pub scopes: Option<Vec<PortalScope>>,
    /// Minted with delegated or express credentials: the session is denied the credit money
    /// actions those credentials are denied.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub delegated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortalTokenError {
    #[error("portal token expired")]
    Expired,
    #[error("invalid portal token")]
    Invalid,
}

impl PortalJwtClaims {
    /// Customer tokens always carry scopes (all of them when `scopes` is `None`); other
    /// resources reject them.
    pub fn new(
        tenant_id: TenantId,
        resource: ResourceAccess,
        scopes: Option<Vec<PortalScope>>,
        expires_in: Option<chrono::Duration>,
    ) -> StoreResult<Self> {
        let scopes = match (&resource, scopes) {
            (ResourceAccess::Customer(_), scopes) => {
                Some(scopes.unwrap_or_else(|| PortalScope::ALL.to_vec()))
            }
            (_, None) => None,
            (_, Some(_)) => {
                return Err(StoreError::InvalidArgument(
                    "scopes only apply to customer portal tokens".into(),
                )
                .into());
            }
        };

        let now = chrono::Utc::now().timestamp();
        Ok(Self {
            iat: now as usize,
            exp: expires_in.map(|d| (now + d.num_seconds()) as usize),
            aud: Some(PORTAL_TOKEN_AUDIENCE.to_string()),
            tenant_id,
            resource,
            scopes,
            delegated: false,
        })
    }

    pub fn with_delegated(mut self, delegated: bool) -> Self {
        self.delegated = delegated;
        self
    }

    pub fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.exp
            .and_then(|exp| chrono::DateTime::from_timestamp(exp as i64, 0))
    }

    pub fn encode(&self, jwt_secret: &SecretString) -> StoreResult<String> {
        crate::jwt::encode(
            &jsonwebtoken::Header::default(),
            self,
            &EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
        )
        .map_err(|_| StoreError::InvalidArgument("failed to generate JWT token".into()).into())
    }

    /// Checks the signature, `exp` and `aud` when present.
    pub fn decode(token: &str, jwt_secret: &SecretString) -> Result<Self, PortalTokenError> {
        let mut validation = jsonwebtoken::Validation::default();
        validation.set_required_spec_claims::<&str>(&[]);
        validation.set_audience(&[PORTAL_TOKEN_AUDIENCE]);
        validation.leeway = 0;

        crate::jwt::decode::<Self>(
            token,
            &DecodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
            &validation,
        )
        .map(|data| data.claims)
        .map_err(|err| {
            log::debug!("Portal token rejected: {err:?}");
            match err.kind() {
                ErrorKind::ExpiredSignature => PortalTokenError::Expired,
                _ => PortalTokenError::Invalid,
            }
        })
    }
}

pub fn generate_portal_token(
    jwt_secret: &SecretString,
    tenant_id: TenantId,
    resource: ResourceAccess,
    scopes: Option<Vec<PortalScope>>,
    expires_in: Option<chrono::Duration>,
    delegated: bool,
) -> StoreResult<String> {
    PortalJwtClaims::new(tenant_id, resource, scopes, expires_in)?
        .with_delegated(delegated)
        .encode(jwt_secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;
    use serde_json::json;

    fn secret() -> SecretString {
        SecretString::new("test-secret".into())
    }

    fn encode_raw(claims: serde_json::Value) -> String {
        crate::jwt::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &EncodingKey::from_secret(b"test-secret"),
        )
        .unwrap()
    }

    fn payload(token: &str) -> serde_json::Value {
        jsonwebtoken::dangerous::insecure_decode(token)
            .unwrap()
            .claims
    }

    #[test]
    fn customer_token_defaults_to_all_scopes_and_emits_aud() {
        let token = generate_portal_token(
            &secret(),
            TenantId::new(),
            ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::hours(1)),
            false,
        )
        .unwrap();

        let raw = payload(&token);
        assert_eq!(raw["scopes"], json!(["read", "manage"]));
        assert_eq!(raw["aud"], json!("meteroid:portal"));

        let claims = PortalJwtClaims::decode(&token, &secret()).unwrap();
        assert_eq!(claims.scopes, Some(PortalScope::ALL.to_vec()));
        assert!(claims.expires_at().is_some());
    }

    #[test]
    fn explicit_scopes_round_trip() {
        let token = generate_portal_token(
            &secret(),
            TenantId::new(),
            ResourceAccess::Customer(CustomerId::new()),
            Some(vec![PortalScope::Read]),
            Some(chrono::Duration::hours(1)),
            false,
        )
        .unwrap();

        assert_eq!(payload(&token)["scopes"], json!(["read"]));
        let claims = PortalJwtClaims::decode(&token, &secret()).unwrap();
        assert_eq!(claims.scopes, Some(vec![PortalScope::Read]));
    }

    #[test]
    fn non_customer_tokens_have_no_scopes() {
        let token = generate_portal_token(
            &secret(),
            TenantId::new(),
            ResourceAccess::Invoice(InvoiceId::new()),
            None,
            None,
            false,
        )
        .unwrap();

        let raw = payload(&token);
        assert!(raw.get("scopes").is_none());
        assert!(raw.get("exp").is_none());
        assert_eq!(raw["aud"], json!("meteroid:portal"));
        assert!(PortalJwtClaims::decode(&token, &secret()).is_ok());

        let with_scopes = generate_portal_token(
            &secret(),
            TenantId::new(),
            ResourceAccess::Invoice(InvoiceId::new()),
            Some(vec![PortalScope::Read]),
            None,
            false,
        );
        assert!(with_scopes.is_err());
    }

    #[test]
    fn legacy_token_without_scopes_or_aud_decodes() {
        let customer_id = CustomerId::new();
        let token = encode_raw(json!({
            "iat": chrono::Utc::now().timestamp(),
            "exp": chrono::Utc::now().timestamp() + 3600,
            "tenant_id": TenantId::new(),
            "resource": { "Customer": customer_id },
        }));

        let claims = PortalJwtClaims::decode(&token, &secret()).unwrap();
        assert_eq!(claims.scopes, None);
        assert_eq!(claims.aud, None);
        assert!(matches!(claims.resource, ResourceAccess::Customer(id) if id == customer_id));
    }

    #[test]
    fn foreign_audience_is_rejected() {
        let token = encode_raw(json!({
            "iat": chrono::Utc::now().timestamp(),
            "exp": chrono::Utc::now().timestamp() + 3600,
            "aud": "web_api",
            "tenant_id": TenantId::new(),
            "resource": { "Customer": CustomerId::new() },
        }));

        assert_eq!(
            PortalJwtClaims::decode(&token, &secret()).unwrap_err(),
            PortalTokenError::Invalid
        );
    }

    #[test]
    fn expired_and_forged_tokens_are_rejected() {
        let token = generate_portal_token(
            &secret(),
            TenantId::new(),
            ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::seconds(-10)),
            false,
        )
        .unwrap();
        assert_eq!(
            PortalJwtClaims::decode(&token, &secret()).unwrap_err(),
            PortalTokenError::Expired
        );

        let valid = generate_portal_token(
            &secret(),
            TenantId::new(),
            ResourceAccess::Customer(CustomerId::new()),
            None,
            Some(chrono::Duration::hours(1)),
            false,
        )
        .unwrap();
        assert_eq!(
            PortalJwtClaims::decode(&valid, &SecretString::new("other".into())).unwrap_err(),
            PortalTokenError::Invalid
        );
        assert_eq!(
            PortalJwtClaims::decode("not-a-jwt", &secret()).unwrap_err(),
            PortalTokenError::Invalid
        );
    }
}
