use axum::http::StatusCode;
use common_domain::ids::{ApiTokenId, OrganizationId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::HeaderMap;
use meteroid_store::Store;

pub const CONNECT_ACCOUNT_HEADER: &str = "x-meteroid-connect-account";

/// A credential failure, as the REST API answers it.
#[derive(Debug)]
pub struct DelegatedAuthError {
    pub status: StatusCode,
    pub message: String,
}

/// Credentials that act on another organization's tenant, honoured by a module (Connect).
#[async_trait::async_trait]
pub trait RestDelegatedAuth: Send + Sync {
    /// Whether this bearer token is one the module issued.
    fn claims_token(&self, token: &str) -> bool;

    /// Refuses the paths the module's tokens may not reach.
    fn check_token_path(&self, path: &str) -> Result<(), DelegatedAuthError>;

    async fn authenticate_token(
        &self,
        store: &Store,
        token: &str,
        headers: &HeaderMap,
    ) -> Result<AuthorizedAsTenant, DelegatedAuthError>;

    /// The tenant a platform's API key acts on, from [`CONNECT_ACCOUNT_HEADER`].
    async fn act_for_account(
        &self,
        store: &Store,
        platform_org_id: OrganizationId,
        actor_id: ApiTokenId,
        headers: &HeaderMap,
    ) -> Result<AuthorizedAsTenant, DelegatedAuthError>;
}
