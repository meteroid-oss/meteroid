//! `GET /api/v2/me` (ADR-0013 §24): who the calling credentials are, for an integrator's first
//! call to check a key: its tenant and organization, the key (never its secret), and for
//! delegated credentials what they may reach, as the module honouring them granted it.
use crate::AppState;
use crate::v2::problem::{Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{ApiTokenId, ConnectedAccountId, OAuthAppId, OrganizationId, TenantId};
use common_grpc::middleware::server::auth::{AuthorizedAsTenant, TenantActor};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::delegated_auth::DelegatedReach;
use meteroid_store::domain::enums::{ApiTokenKind, TenantEnvironmentEnum};
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::{OrganizationsInterface, TenantInterface};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_me))
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TenantEnvironment {
    Production,
    Staging,
    Qa,
    Development,
    Sandbox,
    Demo,
}

impl From<TenantEnvironmentEnum> for TenantEnvironment {
    fn from(environment: TenantEnvironmentEnum) -> Self {
        match environment {
            TenantEnvironmentEnum::Production => TenantEnvironment::Production,
            TenantEnvironmentEnum::Staging => TenantEnvironment::Staging,
            TenantEnvironmentEnum::Qa => TenantEnvironment::Qa,
            TenantEnvironmentEnum::Development => TenantEnvironment::Development,
            TenantEnvironmentEnum::Sandbox => TenantEnvironment::Sandbox,
            TenantEnvironmentEnum::Demo => TenantEnvironment::Demo,
        }
    }
}

/// The tenant every call with these credentials reads and writes.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct MeTenant {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: TenantId,
    pub name: String,
    pub environment: TenantEnvironment,
    /// Its reporting currency (ISO 4217).
    pub currency: String,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct MeOrganization {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: OrganizationId,
    pub name: String,
}

/// `secret_key`: an API key; `oauth_access_token`: an access token issued to a Connect app.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeKeyKind {
    SecretKey,
    OauthAccessToken,
}

/// The credentials of the request, never their secret.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct MeKey {
    pub kind: MeKeyKind,
    /// The API key's id; absent for an access token.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<ApiTokenId>,
    /// The API key's name; absent for an access token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// When the API key was created; absent for an access token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<ApiDateTime>,
    /// When the access token expires; absent for an API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<ApiDateTime>,
}

/// What delegated credentials (a Connect app's access token, or a platform's API key acting for
/// a connected account) may reach. The routes that charge, credit or record money (top-ups,
/// credit grants, wallet writes, budgets, invoice payments) stay refused to them.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct MeDelegation {
    /// The organization that holds the app or the connected account.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub platform_organization: OrganizationId,
    /// The app the access token was issued to.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub oauth_app: Option<OAuthAppId>,
    /// The connected account the platform's key acts for.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub connected_account: Option<ConnectedAccountId>,
    /// The scopes granted (`billing:read`, `customers:write`…); absent when not held to scopes
    /// (an express account, the platform's own).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    /// The path prefixes they may call, under their scopes; absent when not held to paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
}

/// Who the calling credentials are.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Me {
    pub tenant: MeTenant,
    pub organization: MeOrganization,
    pub key: MeKey,
    /// Present for delegated credentials only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation: Option<MeDelegation>,
}

/// Get the calling credentials
///
/// What the key of the request is: its tenant (environment and currency), its organization, the
/// key itself (never its secret) and, for delegated credentials, what they may reach. A first
/// call to check a key works; any credentials may read it, whatever their scopes.
#[utoipa::path(
    get,
    tag = "Credentials",
    path = "/api/v2/me",
    responses(
        (status = 200, description = "The calling credentials", body = Me),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_me(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    reach: Option<Extension<DelegatedReach>>,
) -> Result<Json<Me>, Problem> {
    let tenant = state
        .store
        .find_tenant_by_id_and_organization(auth.tenant_id, auth.organization_id)
        .await?;
    let organization = state
        .store
        .get_organization_by_id(auth.organization_id)
        .await?;
    let reach = reach.map(|Extension(reach)| reach);
    let key = match auth.actor {
        TenantActor::ApiKey(id) => {
            let token = state
                .store
                .find_authenticated_api_token(id)
                .await?
                .filter(|token| token.kind == ApiTokenKind::Secret)
                .ok_or_else(Problem::internal)?;
            MeKey {
                kind: MeKeyKind::SecretKey,
                id: Some(token.id),
                name: Some(token.name),
                created_at: Some(token.created_at.and_utc().into()),
                expires_at: None,
            }
        }
        TenantActor::OAuthApp(_) => MeKey {
            kind: MeKeyKind::OauthAccessToken,
            id: None,
            name: None,
            created_at: None,
            expires_at: reach
                .as_ref()
                .and_then(|reach| reach.expires_at)
                .map(|at| at.and_utc().into()),
        },
        // The REST API authenticates keys and tokens, never a dashboard session.
        TenantActor::User { .. } => {
            return Err(Problem::new(
                http::StatusCode::FORBIDDEN,
                ProblemCode::Forbidden,
                "Not an API credential",
            ));
        }
    };
    let delegation = reach.map(|reach| MeDelegation {
        platform_organization: reach.platform_organization_id,
        oauth_app: reach.oauth_app,
        connected_account: reach.connected_account_id,
        scopes: reach.scopes,
        paths: reach
            .paths
            .map(|paths| paths.into_iter().map(str::to_owned).collect()),
    });
    Ok(Json(Me {
        tenant: MeTenant {
            id: tenant.id,
            name: tenant.name,
            environment: tenant.environment.into(),
            currency: tenant.reporting_currency,
        },
        organization: MeOrganization {
            id: organization.id,
            name: organization.trade_name,
        },
        key,
        delegation,
    }))
}
