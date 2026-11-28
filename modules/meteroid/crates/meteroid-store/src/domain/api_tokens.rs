use chrono::NaiveDateTime;
use common_domain::ids::{ApiTokenId, OrganizationId, TenantId};
use diesel_models::api_tokens::{
    ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow, PublishableKeyRow,
};
use o2o::o2o;

use crate::domain::api_key_permissions::ApiKeyPermissions;
use crate::domain::enums::{ApiTokenKind, TenantEnvironmentEnum};

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRowNew)]
pub struct ApiTokenNew {
    pub name: String,
    pub tenant_id: TenantId,
}

/// How a secret key is restricted. The default is the whole API, forever.
#[derive(Debug, Clone, Default)]
pub struct SecretKeyPolicy {
    pub permissions: Option<ApiKeyPermissions>,
    /// Each key expires this long after it is created; rolling it starts a new one.
    pub lifetime_days: Option<i32>,
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRow)]
pub struct ApiToken {
    pub id: ApiTokenId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub hash: String,
    pub hint: String,
    pub first_used_at: Option<NaiveDateTime>,
    pub last_used_at: Option<NaiveDateTime>,
    #[map(~.into())]
    pub kind: ApiTokenKind,
    /// A publishable key's browser origins; none accepts any.
    pub allowed_origins: Vec<String>,
    /// The whole publishable key, shown again since it is not a secret; none for a secret key.
    pub publishable_key: Option<String>,
    #[from(~.map(ApiKeyPermissions::from_db))]
    pub permissions: Option<ApiKeyPermissions>,
    pub expires_at: Option<NaiveDateTime>,
    pub lifetime_days: Option<i32>,
    /// The key it was rolled into; it keeps working until `expires_at`.
    pub replaced_by_id: Option<ApiTokenId>,
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenValidationRow)]
pub struct ApiTokenValidation {
    pub id: ApiTokenId,
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
    pub hash: String,
    #[from(~.map(ApiKeyPermissions::from_db))]
    pub permissions: Option<ApiKeyPermissions>,
    pub expires_at: Option<NaiveDateTime>,
    #[map(~.into())]
    pub environment: TenantEnvironmentEnum,
    pub keep_last_events: bool,
}

/// A publishable key the client API's public routes accepted.
#[derive(Debug, Clone)]
pub struct PublishableKey {
    pub id: ApiTokenId,
    pub tenant_id: TenantId,
    pub environment: TenantEnvironmentEnum,
    /// The browser origins it is accepted from; none: any.
    pub allowed_origins: Vec<String>,
}

impl From<PublishableKeyRow> for PublishableKey {
    fn from(row: PublishableKeyRow) -> Self {
        PublishableKey {
            id: row.id,
            tenant_id: row.tenant_id,
            environment: row.environment.into(),
            allowed_origins: row.allowed_origins,
        }
    }
}
