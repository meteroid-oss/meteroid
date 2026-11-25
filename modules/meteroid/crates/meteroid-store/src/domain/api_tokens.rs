use chrono::NaiveDateTime;
use common_domain::ids::{ApiTokenId, OrganizationId, TenantId};
use diesel_models::api_tokens::{
    ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow, PublishableKeyRow,
};
use o2o::o2o;

use crate::domain::enums::{ApiTokenKind, TenantEnvironmentEnum};

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRowNew)]
pub struct ApiTokenNew {
    pub name: String,
    pub tenant_id: TenantId,
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRow)]
#[owned_into(ApiTokenRow)]
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
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenValidationRow)]
pub struct ApiTokenValidation {
    pub id: ApiTokenId,
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
    pub hash: String,
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
