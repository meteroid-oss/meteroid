use common_domain::auth::PortalScope;
use common_domain::ids::{CustomerId, TenantId};
use common_grpc::middleware::server::auth::TenantEnv;

/// A customer portal token accepted on the client API.
#[derive(Clone)]
pub struct AuthorizedAsCustomer {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    /// `None` on legacy tokens minted before scopes, which keep full access.
    pub scopes: Option<Vec<PortalScope>>,
    pub tenant_env: TenantEnv,
}
