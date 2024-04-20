use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

#[derive(Debug, o2o)]
#[from_owned(diesel_models::api_tokens::ApiTokenNew)]
#[owned_into(diesel_models::api_tokens::ApiTokenNew)]
pub struct ApiTokenNew {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub hash: String,
    pub hint: String,
}

#[derive(Debug, o2o)]
#[from_owned(diesel_models::api_tokens::ApiToken)]
#[owned_into(diesel_models::api_tokens::ApiToken)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub hash: String,
    pub hint: String,
}
