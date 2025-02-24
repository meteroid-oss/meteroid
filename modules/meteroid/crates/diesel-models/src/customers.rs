use chrono::NaiveDateTime;
use uuid::Uuid;

use common_domain::ids::{CustomerId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRow {
    pub id: CustomerId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub billing_config: serde_json::Value,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Uuid,
    pub archived_by: Option<Uuid>,
}

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerForDisplayRow {
    pub id: CustomerId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub billing_config: serde_json::Value,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Uuid,
    #[diesel(select_expression = crate::schema::invoicing_entity::local_id)]
    #[diesel(select_expression_type = crate::schema::invoicing_entity::local_id)]
    pub invoicing_entity_local_id: String,
}

#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBriefRow {
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRowNew {
    pub id: CustomerId,
    pub name: String,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub billing_config: serde_json::Value,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Uuid,
    // for seed, else default to None
    pub created_at: Option<NaiveDateTime>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRowPatch {
    pub id: CustomerId,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: Option<i32>,
    pub currency: Option<String>,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Option<Uuid>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(treat_none_as_null = true)]
pub struct CustomerRowUpdate {
    pub id: CustomerId,
    pub name: String,
    pub billing_config: serde_json::Value,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub updated_by: Uuid,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Uuid,
}
