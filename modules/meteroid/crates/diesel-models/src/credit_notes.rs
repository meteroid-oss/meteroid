use chrono::NaiveDateTime;

use crate::enums::{CreditNoteOriginEnum, CreditNoteStatus, CreditTypeEnum, EInvoicingStatusEnum};
use common_domain::ids::{
    CreditNoteId, CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId, StoredDocumentId,
    SubscriptionId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::credit_note)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditNoteRow {
    pub id: CreditNoteId,
    pub credit_note_number: String,
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
    pub line_items: serde_json::Value,
    pub tax_breakdown: serde_json::Value,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub conn_meta: Option<serde_json::Value>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub credit_type: CreditTypeEnum,
    /// Trigger-maintained analytics ingestion marker; never written by app code.
    pub store_committed_at: NaiveDateTime,
    pub custom_properties: serde_json::Value,
    /// Whether the structured e-invoice was produced; see `InvoiceRow`.
    pub einvoicing_status: Option<EInvoicingStatusEnum>,
    pub einvoicing_report: Option<serde_json::Value>,
    pub xml_document_id: Option<StoredDocumentId>,
    pub debt_cancelled_amount_cents: i64,
    /// Main added its credited amount to the scalar balance carried into the ledger.
    pub credited_retired_balance: bool,
    /// Issued by voiding its invoice, whose own reversal carries the revenue it reverses.
    pub voids_invoice: bool,
    /// The transaction that last wrote the row, trigger-set; the analytics projector's keyset.
    pub store_xid: Option<i64>,
    pub origin: CreditNoteOriginEnum,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::credit_note)]
pub struct CreditNoteRowNew {
    pub id: CreditNoteId,
    pub credit_note_number: String,
    pub status: CreditNoteStatus,
    pub credit_type: CreditTypeEnum,
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
    pub debt_cancelled_amount_cents: i64,
    pub line_items: serde_json::Value,
    pub tax_breakdown: serde_json::Value,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
    pub invoicing_entity_id: InvoicingEntityId,
    pub custom_properties: serde_json::Value,
    pub voids_invoice: bool,
    pub origin: CreditNoteOriginEnum,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::credit_note)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditNoteRowPatch {
    pub status: Option<CreditNoteStatus>,
    pub finalized_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub pdf_document_id: Option<StoredDocumentId>,
}
