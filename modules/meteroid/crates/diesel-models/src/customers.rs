use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRow {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
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
}

#[derive(Clone, Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerBriefRow {
    pub id: Uuid,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRowNew {
    pub id: Uuid,
    pub name: String,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
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
    pub id: Uuid,
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

#[derive(AsChangeset, Debug)]
#[diesel(table_name = crate::schema::customer)]
pub struct CustomerRowAsChangeset {
    pub name: String,
    pub billing_config: Option<serde_json::Value>,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
}

#[derive(Debug)]
pub enum CustomerUpdate {
    UpdateDetails {
        name: Option<String>,
        alias: Option<String>,
        email: Option<Option<String>>,
        invoicing_email: Option<Option<String>>,
    },
    UpdateAddress {
        billing_address: Option<serde_json::Value>,
        shipping_address: Option<serde_json::Value>,
    },
    UpdateBalance {
        balance_value_cents: i32,
        currency: String,
    },
}
