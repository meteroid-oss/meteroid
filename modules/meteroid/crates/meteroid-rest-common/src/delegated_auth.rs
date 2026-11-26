use axum::http::StatusCode;
use common_domain::ids::{ApiTokenId, ConnectedAccountId, OAuthAppId, OrganizationId};
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

/// Who delegated credentials act as, and what they may reach.
#[derive(Clone)]
pub struct Delegated {
    pub authorized: AuthorizedAsTenant,
    pub reach: DelegatedReach,
}

/// What delegated credentials may reach, as the module honouring them grants it; in the request
/// extensions beside `DelegatedCredentials` (`GET /api/v2/me` reads it). The routes that charge,
/// credit or record money stay refused to them whatever it says.
#[derive(Debug, Clone)]
pub struct DelegatedReach {
    /// The organization the credentials were granted by.
    pub platform_organization_id: OrganizationId,
    /// An app's access token: the app.
    pub oauth_app: Option<OAuthAppId>,
    /// A platform's API key acting for one of its connected accounts: the account.
    pub connected_account_id: Option<ConnectedAccountId>,
    /// The scopes granted; none when they are not held to scopes (an express account).
    pub scopes: Option<Vec<String>>,
    /// The path prefixes they may call; none when they are not held to paths.
    pub paths: Option<Vec<&'static str>>,
    /// When an access token expires.
    pub expires_at: Option<chrono::NaiveDateTime>,
}

/// Credentials that act on another organization's tenant, honoured by a module (Connect).
#[async_trait::async_trait]
pub trait RestDelegatedAuth: Send + Sync {
    /// Whether this bearer token is one the module issued.
    fn claims_token(&self, token: &str) -> bool;

    /// Refuses the paths the module's tokens may not reach.
    fn check_token_path(&self, path: &str) -> Result<(), DelegatedAuthError>;

    /// Finds the tenant for a token the module issued, and what the token may reach. Fails if
    /// the token's scopes don't cover the request.
    async fn authenticate_token(
        &self,
        store: &Store,
        token: &str,
        req: &Parts,
    ) -> Result<Delegated, DelegatedAuthError>;

    /// Finds the tenant a platform's API key is acting as, from [`CONNECT_ACCOUNT_HEADER`], and
    /// what it may reach there. Fails if the account's scopes don't cover the request.
    async fn act_for_account(
        &self,
        store: &Store,
        platform_org_id: OrganizationId,
        actor_id: ApiTokenId,
        req: &Parts,
    ) -> Result<Delegated, DelegatedAuthError>;
}
