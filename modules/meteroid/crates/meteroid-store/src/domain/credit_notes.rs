use super::enums::CreditNoteStatus;
use super::invoice_lines::LineItem;
use super::invoices::{InlineCustomer, InlineInvoicingEntity, TaxBreakdownItem};
use crate::domain::Customer;
use crate::domain::connectors::ConnectionMeta;
use crate::errors::{StoreError, StoreErrorReport};
use chrono::NaiveDateTime;
use common_domain::ids::{
    BaseId, CreditNoteId, CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId,
    StoredDocumentId, SubscriptionId, TenantId,
};
use diesel_models::credit_notes::{CreditNoteRow, CreditNoteRowNew};
use o2o::o2o;

#[derive(Debug, Clone, o2o, PartialEq, Eq)]
#[try_from_owned(CreditNoteRow, StoreErrorReport)]
pub struct CreditNote {
    pub id: CreditNoteId,
    pub credit_note_number: String,
    #[from(~.into())]
    pub status: CreditNoteStatus,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub finalized_at: Option<NaiveDateTime>,
    pub voided_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    pub plan_version_id: Option<PlanVersionId>,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub subtotal: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[from(serde_json::from_value(~).map_err(| e | {
    StoreError::SerdeError("Failed to deserialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub pdf_document_id: Option<StoredDocumentId>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub conn_meta: Option<ConnectionMeta>,
    pub invoicing_entity_id: InvoicingEntityId,
}

#[derive(Debug, Clone, o2o)]
#[owned_try_into(CreditNoteRowNew, StoreErrorReport)]
#[ghosts(id: {CreditNoteId::new()})]
pub struct CreditNoteNew {
    pub credit_note_number: String,
    #[into(~.into())]
    pub status: CreditNoteStatus,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub invoice_id: InvoiceId,
    pub invoice_number: String,
    pub plan_version_id: Option<PlanVersionId>,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub subtotal: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub refunded_amount_cents: i64,
    pub credited_amount_cents: i64,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize line_items".to_string(), e)
    }) ?)]
    pub line_items: Vec<LineItem>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize tax_breakdown".to_string(), e)
    }) ?)]
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub reason: Option<String>,
    pub memo: Option<String>,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize customer_details".to_string(), e)
    }) ?)]
    pub customer_details: InlineCustomer,
    #[into(serde_json::to_value(& ~).map_err(| e | {
    StoreError::SerdeError("Failed to serialize seller_details".to_string(), e)
    }) ?)]
    pub seller_details: InlineInvoicingEntity,
    pub invoicing_entity_id: InvoicingEntityId,
    #[ghost({None})]
    pub finalized_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailedCreditNote {
    pub credit_note: CreditNote,
    pub customer: Customer,
}
