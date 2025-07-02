use chrono::NaiveDateTime;
use uuid::Uuid;

use common_domain::ids::{
    BankAccountId, ConnectorId, CustomerId, CustomerPaymentMethodId, InvoicingEntityId, TenantId,
};
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
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i64,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub archived_by: Option<Uuid>,
    pub bank_account_id: Option<BankAccountId>,
    pub current_payment_method_id: Option<CustomerPaymentMethodId>,
    pub card_provider_id: Option<ConnectorId>,
    pub direct_debit_provider_id: Option<ConnectorId>,
    pub vat_number: Option<String>,
    pub custom_vat_rate: Option<i32>,
    pub invoicing_emails: Vec<Option<String>>,
    pub conn_meta: Option<serde_json::Value>,
    //  logo_url -> Nullable<Text>,
    //  website_url -> Nullable<Text>,
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
    pub alias: Option<String>,
    pub phone: Option<String>,
    pub balance_value_cents: i64,
    pub currency: String,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: InvoicingEntityId,
    // for seed, else default to None
    pub created_at: Option<NaiveDateTime>,
    pub billing_email: Option<String>,
    pub bank_account_id: Option<BankAccountId>,
    pub current_payment_method_id: Option<CustomerPaymentMethodId>,
    pub card_provider_id: Option<ConnectorId>,
    pub direct_debit_provider_id: Option<ConnectorId>,
    pub vat_number: Option<String>,
    pub custom_vat_rate: Option<i32>,
    pub invoicing_emails: Vec<Option<String>>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomerRowPatch {
    pub id: CustomerId,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Option<Vec<Option<String>>>,
    pub phone: Option<String>,
    pub balance_value_cents: Option<i64>,
    pub currency: Option<String>,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<Option<String>>,
    pub custom_vat_rate: Option<Option<i32>>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::customer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(treat_none_as_null = true)]
pub struct CustomerRowUpdate {
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<Option<String>>,
    pub phone: Option<String>,
    pub currency: String,
    pub updated_by: Uuid,
    pub billing_address: Option<serde_json::Value>,
    pub shipping_address: Option<serde_json::Value>,
    pub invoicing_entity_id: InvoicingEntityId,
}
