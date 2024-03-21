use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::schema::customer;
use diesel::prelude::*;
use diesel::{AsChangeset, Identifiable, Insertable, Queryable};
#[derive(Clone, Debug, Identifiable, Queryable)]
#[diesel(table_name = crate::schema::customer)]
pub struct Customer {
    pub id: Uuid,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub updated_by: Option<Uuid>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub billing_config: Option<serde_json::Value>,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub balance_currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::customer)]
pub struct CustomerNew {
    pub id: Uuid,
    pub name: String,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub billing_config: Option<serde_json::Value>,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub balance_currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    // for seed
    pub created_at: Option<NaiveDateTime>,
}

#[derive(AsChangeset, Debug)]
#[diesel(table_name = crate::schema::customer)]
pub struct CustomerAsChangeset {
    pub name: String,
    pub billing_config: Option<serde_json::Value>,
    pub alias: Option<String>,
    pub email: Option<String>,
    pub invoicing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i32,
    pub balance_currency: String,
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
        balance_currency: String,
    },
}
