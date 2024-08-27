use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

use diesel_models::api_tokens::{ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow};

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRowNew)]
pub struct ApiTokenNew {
    pub name: String,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenRow)]
#[owned_into(ApiTokenRow)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub hash: String,
    pub hint: String,
}

#[derive(Debug, o2o)]
#[from_owned(ApiTokenValidationRow)]
pub struct ApiTokenValidation {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub organization_id: Uuid,
    pub hash: String,
}
