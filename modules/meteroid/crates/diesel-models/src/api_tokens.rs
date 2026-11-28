use chrono::NaiveDateTime;

use crate::enums::{ApiTokenKindEnum, TenantEnvironmentEnum};
use common_domain::ids::{ApiTokenId, OrganizationId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Queryable, Identifiable)]
#[diesel(table_name = crate::schema::api_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiTokenRow {
    pub id: ApiTokenId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub hash: String,
    pub hint: String,
    pub first_used_at: Option<NaiveDateTime>,
    pub last_used_at: Option<NaiveDateTime>,
    pub kind: ApiTokenKindEnum,
    #[diesel(deserialize_as = crate::extend::non_null_array::NonNullArray<String>)]
    pub allowed_origins: Vec<String>,
    pub publishable_key: Option<String>,
    pub permissions: Option<serde_json::Value>,
    pub expires_at: Option<NaiveDateTime>,
    pub lifetime_days: Option<i32>,
    pub replaced_by_id: Option<ApiTokenId>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::api_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiTokenRowNew {
    pub id: ApiTokenId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub tenant_id: TenantId,
    pub hash: String,
    pub hint: String,
    pub kind: ApiTokenKindEnum,
    pub allowed_origins: Vec<String>,
    pub publishable_key: Option<String>,
    pub permissions: Option<serde_json::Value>,
    pub expires_at: Option<NaiveDateTime>,
    pub lifetime_days: Option<i32>,
}

// ApiTokenValidationRow: secret keys only (`find_by_id` filters on the kind).
#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::api_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiTokenValidationRow {
    pub id: ApiTokenId,
    pub tenant_id: TenantId,
    pub hash: String,
    pub permissions: Option<serde_json::Value>,
    pub expires_at: Option<NaiveDateTime>,
    #[diesel(select_expression = crate::schema::tenant::organization_id)]
    #[diesel(select_expression_type = crate::schema::tenant::organization_id)]
    pub organization_id: OrganizationId,
    #[diesel(select_expression = crate::schema::tenant::environment)]
    #[diesel(select_expression_type = crate::schema::tenant::environment)]
    pub environment: TenantEnvironmentEnum,
    #[diesel(select_expression = crate::schema::tenant::keep_last_events)]
    #[diesel(select_expression_type = crate::schema::tenant::keep_last_events)]
    pub keep_last_events: bool,
}

/// A publishable key as the client API's public routes authenticate it.
#[derive(Debug, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::api_token)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PublishableKeyRow {
    pub id: ApiTokenId,
    pub tenant_id: TenantId,
    pub publishable_key: Option<String>,
    #[diesel(deserialize_as = crate::extend::non_null_array::NonNullArray<String>)]
    pub allowed_origins: Vec<String>,
    #[diesel(select_expression = crate::schema::tenant::environment)]
    #[diesel(select_expression_type = crate::schema::tenant::environment)]
    pub environment: TenantEnvironmentEnum,
}
