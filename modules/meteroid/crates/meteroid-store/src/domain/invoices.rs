use super::enums::{
    InvoiceExternalStatusEnum, InvoiceStatusEnum, InvoiceType, InvoicingProviderEnum,
};
use crate::domain::invoice_lines::LineItem;
use crate::domain::{Address, Customer, PlanVersionLatest};
use crate::errors::StoreError;
use chrono::{NaiveDate, NaiveDateTime};
use diesel_models::invoices::DetailedInvoiceRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoices::InvoiceRowLinesPatch;
use diesel_models::invoices::InvoiceRowNew;
use diesel_models::invoices::InvoiceWithCustomerRow;
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, o2o, PartialEq, Eq)]
#[try_from_owned(InvoiceRow, Report < StoreError >)]
pub struct Invoice {
    pub id: Uuid,
    #[from(~.into())]
    pub status: InvoiceStatusEnum,
    #[from(~.map(| x | x.into()))]
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Option<Uuid>,
    pub currency: String,
    pub external_invoice_id: Option<String>,
    pub invoice_number: String,
    #[from(~.into())]
    pub invoicing_provider: InvoicingProviderEnum,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<Uuid>,
    #[from(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_rate: i32,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub memo: Option<String>,
    pub local_id: String,
    pub due_at: Option<NaiveDateTime>,
    pub plan_name: Option<String>,

    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
}

#[derive(Debug, o2o)]
#[owned_try_into(InvoiceRowNew, Report < StoreError >)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct InvoiceNew {
    #[into(~.into())]
    pub status: InvoiceStatusEnum,
    #[into(~.map(| x | x.into()))]
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Option<Uuid>,
    pub currency: String,
    pub external_invoice_id: Option<String>,
    pub invoice_number: String,
    #[into(~.into())]
    pub invoicing_provider: InvoicingProviderEnum,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<Uuid>,
    #[into(~.into())]
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_rate: i32,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub memo: Option<String>,
    pub local_id: String,
    pub due_at: Option<NaiveDateTime>,
    pub plan_name: Option<String>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
}

#[derive(Debug, o2o)]
#[owned_try_into(InvoiceRowLinesPatch, Report < StoreError >)]
pub struct InvoiceLinesPatch {
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    pub amount_due: i64,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub total: i64,
    pub tax_amount: i64,
}

impl InvoiceLinesPatch {
    pub fn from_invoice_and_lines(invoice: &Invoice, line_items: Vec<LineItem>) -> Self {
        let subtotal = line_items.iter().fold(0, |acc, x| acc + x.subtotal);
        let tax_amount = subtotal * invoice.tax_rate as i64 / 100;
        let total = subtotal + tax_amount; // TODO discounts etc
        let already_paid = invoice.total - invoice.amount_due;
        let amount_due = total - already_paid;
        let subtotal_recurring = line_items
            .iter()
            .filter(|x| x.metric_id.is_none())
            .fold(0, |acc, x| acc + x.subtotal);

        InvoiceLinesPatch {
            line_items: line_items,
            amount_due: amount_due,
            subtotal: subtotal,
            subtotal_recurring: subtotal_recurring,
            total: total,
            tax_amount: tax_amount,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct InlineCustomer {
    pub id: Uuid,
    pub name: String,
    pub billing_address: Option<Address>,
    pub snapshot_at: NaiveDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceWithCustomer {
    pub invoice: Invoice,
    pub customer: Customer,
}

impl TryFrom<InvoiceWithCustomerRow> for InvoiceWithCustomer {
    type Error = Report<StoreError>;

    fn try_from(value: InvoiceWithCustomerRow) -> Result<Self, Self::Error> {
        Ok(InvoiceWithCustomer {
            invoice: value.invoice.try_into()?,
            customer: value.customer.try_into()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedInvoice {
    pub invoice: Invoice,
    pub customer: Customer,
    pub plan: Option<PlanVersionLatest>,
}

impl TryFrom<DetailedInvoiceRow> for DetailedInvoice {
    type Error = Report<StoreError>;

    fn try_from(value: DetailedInvoiceRow) -> Result<Self, Self::Error> {
        Ok(DetailedInvoice {
            invoice: value.invoice.try_into()?,
            customer: value.customer.try_into()?,
            plan: value.plan.map(|x| x.into()),
        })
    }
}
