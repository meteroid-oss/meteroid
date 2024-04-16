use super::enums::{
    InvoiceExternalStatusEnum, InvoiceStatusEnum, InvoiceType, InvoicingProviderEnum,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use diesel_models::invoices::Invoice as DieselInvoice;
use diesel_models::invoices::InvoiceNew as DieselInvoiceNew;
use o2o::o2o;
use uuid::Uuid;
#[derive(Debug, Clone, o2o)]
#[from_owned(DieselInvoice)]
pub struct Invoice {
    pub id: Uuid,
    #[from(~.into())]
    pub status: InvoiceStatusEnum,
    #[from(~.map(|x| x.into()))]
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Uuid,
    pub currency: String,
    pub days_until_due: Option<i32>,
    pub external_invoice_id: Option<String>,
    pub invoice_id: Option<String>,
    #[from(~.into())]
    pub invoicing_provider: InvoicingProviderEnum,
    pub line_items: serde_json::Value,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<DateTime<Utc>>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub amount_cents: Option<i64>,
    pub plan_version_id: Option<Uuid>,
    #[from(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
}

#[derive(Debug, o2o)]
#[owned_into(DieselInvoiceNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct InvoiceNew {
    #[into(~.into())]
    pub status: InvoiceStatusEnum,
    #[into(~.map(|x| x.into()))]
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Uuid,
    pub currency: String,
    pub days_until_due: Option<i32>,
    pub external_invoice_id: Option<String>,
    pub invoice_id: Option<String>,
    #[into(~.into())]
    pub invoicing_provider: InvoicingProviderEnum,
    pub line_items: serde_json::Value,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<DateTime<Utc>>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub amount_cents: Option<i64>,
    pub plan_version_id: Option<Uuid>,
    #[into(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
}
