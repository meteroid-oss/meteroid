use super::enums::{
    InvoiceExternalStatusEnum, InvoiceStatusEnum, InvoiceType, InvoicingProviderEnum,
};
use crate::domain::Customer;
use crate::errors::StoreError;
use chrono::{NaiveDate, NaiveDateTime};
use diesel_models::invoices::Invoice as DieselInvoice;
use diesel_models::invoices::InvoiceNew as DieselInvoiceNew;
use diesel_models::invoices::InvoiceWithCustomer as DieselInvoiceBrief;
use diesel_models::invoices::InvoiceWithPlanDetails as DieselInvoiceWithPlanDetails;
use error_stack::Report;
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
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
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
    pub last_issue_attempt_at: Option<NaiveDateTime>,
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
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub amount_cents: Option<i64>,
    pub plan_version_id: Option<Uuid>,
    #[into(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, o2o)]
#[from_owned(DieselInvoiceWithPlanDetails)]
pub struct InvoiceWithPlanDetails {
    pub id: uuid::Uuid,
    #[from(~.into())]
    pub status: InvoiceStatusEnum,
    #[from(~.map(|x| x.into()))]
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
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
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub amount_cents: Option<i64>,
    pub customer_name: String,
    pub plan_name: String,
    pub plan_external_id: String,
    pub plan_version: i32,
}

#[derive(Debug, Clone)]
pub struct InvoiceWithCustomer {
    pub invoice: Invoice,
    pub customer: Customer,
}

impl TryFrom<diesel_models::invoices::InvoiceWithCustomer> for InvoiceWithCustomer {
    type Error = Report<StoreError>;

    fn try_from(value: DieselInvoiceBrief) -> Result<Self, Self::Error> {
        Ok(InvoiceWithCustomer {
            invoice: value.invoice.into(),
            customer: value.customer.try_into()?,
        })
    }
}
