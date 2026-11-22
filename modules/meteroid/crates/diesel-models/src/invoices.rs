use crate::enums::{EInvoicingStatusEnum, InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use chrono::NaiveDate;
use chrono::NaiveDateTime;

use crate::customers::CustomerRow;
use crate::plan_versions::PlanVersionRowOverview;
use common_domain::ids::{
    CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId, StoredDocumentId, SubscriptionId,
    TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceRow {
    pub id: InvoiceId,
    pub status: InvoiceStatusEnum,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub line_items: serde_json::Value,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub total: i64,
    pub plan_version_id: Option<PlanVersionId>,
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub net_terms: i32,
    pub memo: Option<String>,
    pub reference: Option<String>,
    pub invoice_number: String,
    pub tax_amount: i64,
    pub subtotal_recurring: i64,
    pub plan_name: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub customer_details: serde_json::Value,
    pub amount_due: i64,
    pub subtotal: i64,
    pub applied_credits: i64,
    pub seller_details: serde_json::Value,
    pub xml_document_id: Option<StoredDocumentId>,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub conn_meta: Option<serde_json::Value>,
    pub auto_advance: bool,
    pub issued_at: Option<NaiveDateTime>,
    pub payment_status: InvoicePaymentStatus,
    pub paid_at: Option<NaiveDateTime>,
    pub discount: i64,
    pub purchase_order: Option<String>,
    pub coupons: serde_json::Value,
    pub tax_breakdown: serde_json::Value,
    pub manual: bool,
    pub voided_at: Option<NaiveDateTime>,
    pub marked_as_uncollectible_at: Option<NaiveDateTime>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub parent_invoice_id: Option<InvoiceId>,
    pub consolidated_into_invoice_id: Option<InvoiceId>,
    pub annexes: serde_json::Value,
    /// Trigger-maintained analytics ingestion marker; never written by app code.
    pub store_committed_at: NaiveDateTime,
    pub custom_properties: serde_json::Value,
    /// Cycle these lines were computed for; a refresh gates on it, not the subscription's
    /// current cycle. Null for rows predating the column.
    pub cycle_index: Option<i32>,
    /// Set when an operator edited this draft's billing content (lines or discount); a manual
    /// finalize then keeps the stored lines instead of settling with a refresh.
    pub manually_edited_at: Option<NaiveDateTime>,
    /// Set when an operator edited this draft's billing identity, opting it out of tracking the
    /// live customer record. Null means `customer_details` is a cache to be refreshed on read.
    pub customer_details_overridden_at: Option<NaiveDateTime>,
    /// Start of the billing period this invoice settles. This is the period key (dedup,
    /// plan-change guards); `invoice_date` is the emission date, stamped at finalization.
    pub billing_period_start: NaiveDate,
    /// Whether the structured e-invoice was produced. `None` when the entity had
    /// not opted in; see `einvoicing_report` for why a `Failed` one did not.
    pub einvoicing_status: Option<EInvoicingStatusEnum>,
    pub einvoicing_report: Option<serde_json::Value>,
    /// The prices this invoice bills were quoted tax-included. Every stored amount is net
    /// all the same — the tax was carved out of the quoted price, not added to it — so this
    /// only tells a recompute which way to read fresh prices, and a renderer what to say.
    pub tax_inclusive: bool,
    /// The collection decision taken at finalization: when an automatic charge was requested.
    /// NULL = pay-link invoice. Written only by `set_auto_charge_requested`.
    pub auto_charge_requested_at: Option<NaiveDateTime>,
    /// Promotional credit taken off the charges before tax, net like the discount.
    pub promotional_credits: i64,
    /// The transaction that last wrote the row, trigger-set; the analytics projector's keyset.
    pub store_xid: Option<i64>,
    /// The first withdrawal by a dispute still standing on its payments; the invoice stays paid.
    pub payment_disputed_at: Option<NaiveDateTime>,
    /// Set when a sandbox seed replayed this invoice from past billing: never collected nor sent.
    pub delivery_held_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When a draft is finalized once its grace ends; written next to its FinalizeInvoice event.
    pub finalize_after: Option<NaiveDateTime>,
    /// When its payment is tried again; written next to its RetryPayment event.
    pub next_payment_attempt_at: Option<NaiveDateTime>,
    /// Closed by an operator deleting the draft: its lines forgive what they billed (R-124).
    pub waived_at: Option<NaiveDateTime>,
}

/// Freezes the resolved identity onto an invoice at finalization, so the issued document keeps
/// what was rendered rather than continuing to track the live records.
#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceRowIdentityPatch {
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceRowLinesPatch {
    pub line_items: serde_json::Value,
    pub amount_due: i64,
    pub subtotal: i64,
    pub discount: i64,
    pub subtotal_recurring: i64,
    pub total: i64,
    pub tax_amount: i64,
    pub applied_credits: i64,
    pub promotional_credits: i64,
    pub tax_breakdown: serde_json::Value,
    pub annexes: serde_json::Value,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceRowPatch {
    pub line_items: serde_json::Value,
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
    pub discount: i64,
    pub memo: Option<String>,
    pub reference: Option<String>,
    pub purchase_order: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub promotional_credits: i64,
    pub tax_breakdown: serde_json::Value,
    pub custom_properties: serde_json::Value,
    /// `None` leaves the column untouched (AsChangeset skips it); `Some` replaces the snapshot.
    pub annexes: Option<serde_json::Value>,
    /// `None` leaves the column untouched (AsChangeset skips it); `Some` stamps the edit.
    pub manually_edited_at: Option<NaiveDateTime>,
    /// Nested option so the override can be cleared, not just stamped: `None` leaves the column
    /// alone, `Some(None)` writes NULL (back to tracking the customer), `Some(Some(ts))` stamps it.
    pub customer_details_overridden_at: Option<Option<NaiveDateTime>>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::invoice)]
pub struct InvoiceRowNew {
    pub id: InvoiceId,
    pub status: InvoiceStatusEnum,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub invoice_number: String,
    pub line_items: serde_json::Value,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<PlanVersionId>,
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub applied_credits: i64,
    pub promotional_credits: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub memo: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub plan_name: Option<String>,
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
    pub auto_advance: bool,
    pub payment_status: InvoicePaymentStatus,
    pub coupons: serde_json::Value,
    pub discount: i64,
    pub purchase_order: Option<String>,
    pub tax_breakdown: serde_json::Value,
    pub manual: bool,
    pub invoicing_entity_id: InvoicingEntityId,
    pub parent_invoice_id: Option<InvoiceId>,
    pub consolidated_into_invoice_id: Option<InvoiceId>,
    pub annexes: serde_json::Value,
    pub custom_properties: serde_json::Value,
    /// Cycle these lines were computed for; see `InvoiceRow::cycle_index`.
    pub cycle_index: Option<i32>,
    /// See `InvoiceRow::billing_period_start`.
    pub billing_period_start: NaiveDate,
    /// See `InvoiceRow::tax_inclusive`.
    pub tax_inclusive: bool,
}

/// An issued invoice still owed, with only what receivables and pending pack credit read.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UnsettledInvoiceRow {
    pub id: InvoiceId,
    pub invoice_number: String,
    pub currency: String,
    pub subscription_id: Option<SubscriptionId>,
    pub amount_due: i64,
    pub due_at: Option<NaiveDateTime>,
    pub payment_status: InvoicePaymentStatus,
    pub line_items: serde_json::Value,
}

/// The lines a subscription contributed to the consolidated invoice they were merged into.
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ConsolidatedMemberLinesRow {
    pub consolidated_into_invoice_id: Option<InvoiceId>,
    pub line_items: serde_json::Value,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceWithCustomerRow {
    #[diesel(embed)]
    pub invoice: InvoiceRow,
    #[diesel(embed)]
    pub customer: CustomerRow,
}

#[derive(Debug)]
pub struct InvoiceLockRow {
    pub lock: crate::lock::CustomerLock,
    pub invoice: InvoiceRow,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DetailedInvoiceRow {
    #[diesel(embed)]
    pub invoice: InvoiceRow,
    #[diesel(embed)]
    pub customer: CustomerRow,
    #[diesel(embed)]
    pub plan: Option<PlanVersionRowOverview>,
}
