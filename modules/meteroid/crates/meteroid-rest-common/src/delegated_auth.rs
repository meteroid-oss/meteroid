use axum::http::StatusCode;
use common_domain::ids::{ApiTokenId, OrganizationId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::request::Parts;
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

    /// Finds the tenant for a token the module issued. Fails if the token's scopes don't
    /// cover the request.
    async fn authenticate_token(
        &self,
        store: &Store,
        token: &str,
        req: &Parts,
    ) -> Result<AuthorizedAsTenant, DelegatedAuthError>;

    /// Finds the tenant a platform's API key is acting as, from [`CONNECT_ACCOUNT_HEADER`].
    /// Fails if the account's scopes don't cover the request.
    async fn act_for_account(
        &self,
        store: &Store,
        platform_org_id: OrganizationId,
        actor_id: ApiTokenId,
        req: &Parts,
    ) -> Result<AuthorizedAsTenant, DelegatedAuthError>;
}
