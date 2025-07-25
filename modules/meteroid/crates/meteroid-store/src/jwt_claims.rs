use crate::StoreResult;
use crate::errors::StoreError;
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId, TenantId};
use secrecy::{ExposeSecret, SecretString};
// todo reuse in common-grpc as well

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub enum ResourceAccess {
    SubscriptionCheckout(SubscriptionId),
    // OneTimeCheckout
    Customer(CustomerId),
    Invoice(InvoiceId),
}
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct PortalJwtClaims {
    iat: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<usize>,
    pub tenant_id: TenantId,
    pub resource: ResourceAccess,
}

impl PortalJwtClaims {
    pub fn new(tenant_id: TenantId, resource: ResourceAccess) -> Self {
        Self {
            iat: chrono::Utc::now().timestamp() as usize,
            exp: None,
            tenant_id,
            resource,
        }
    }
}

pub fn generate_portal_token(
    jwt_secret: &SecretString,
    tenant_id: TenantId,
    resource: ResourceAccess,
) -> StoreResult<String> {
    let claims = serde_json::to_value(PortalJwtClaims::new(tenant_id, resource))
        .map_err(|err| StoreError::SerdeError("failed to generate JWT token".into(), err))?;

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
    )
    .map_err(|_| StoreError::InvalidArgument("failed to generate JWT token".into()))?;
    Ok(token)
}
