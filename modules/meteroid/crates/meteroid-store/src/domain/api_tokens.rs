use chrono::NaiveDateTime;
use common_domain::ids::{OrganizationId, TenantId};
use diesel_models::api_tokens::{ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow};
use o2o::o2o;
use uuid::Uuid;

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRowNew)]
pub struct ApiTokenNew {
    pub name: String,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRow)]
#[owned_into(ApiTokenRow)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub hash: String,
    pub hint: String,
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenValidationRow)]
pub struct ApiTokenValidation {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
    pub hash: String,
}
