use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId, TenantId};
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
