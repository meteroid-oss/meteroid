use common_domain::auth::PortalScope;
use common_domain::ids::{ApiTokenId, CustomerId, TenantId};
use common_grpc::middleware::server::auth::TenantEnv;
use meteroid_store::jwt_claims::ClientClaims;

/// A customer portal token accepted on the client API.
#[derive(Clone)]
pub struct AuthorizedAsCustomer {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    /// `None` on legacy tokens minted before scopes, which keep full access.
    pub scopes: Option<Vec<PortalScope>>,
    pub tenant_env: TenantEnv,
    /// What the token may name in a check or a snapshot; empty means the customer only.
    pub claims: ClientClaims,
}

/// A publishable key accepted on one of the client API's public routes: it names a tenant, no
/// customer and no actor, and reaches nothing else.
#[derive(Clone)]
pub struct AuthorizedAsPublishable {
    pub tenant_id: TenantId,
    pub key_id: ApiTokenId,
    pub tenant_env: TenantEnv,
}
