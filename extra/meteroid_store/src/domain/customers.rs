use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;
#[derive(Clone, Debug, o2o)]
#[from_owned(diesel_models::customers::Customer)]
#[owned_into(diesel_models::customers::Customer)]
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

#[derive(Clone, Debug, o2o)]
#[owned_into(diesel_models::customers::CustomerNew)]
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
    pub billing_address: Option<serde_json::Value>, // TODO avoid json
    pub shipping_address: Option<serde_json::Value>,
}
