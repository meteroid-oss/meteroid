use chrono::NaiveDateTime;

use crate::enums::CreditNoteStatus;
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
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::credit_note)]
pub struct CreditNoteRowNew {
    pub id: CreditNoteId,
    pub credit_note_number: String,
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
    pub line_items: serde_json::Value,
    pub tax_breakdown: serde_json::Value,
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
    pub invoicing_entity_id: InvoicingEntityId,
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
