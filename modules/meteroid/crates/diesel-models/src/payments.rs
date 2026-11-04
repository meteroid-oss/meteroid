use crate::customer_payment_methods::CustomerPaymentMethodRow;
use crate::enums::{
    DeclineKindEnum, PaymentStatusEnum, PaymentTypeEnum, RefundModeEnum, ReversalKindEnum,
};
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{
    CheckoutSessionId, CreditNoteId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
    InvoiceId, PaymentTransactionId, PlanVersionId, StoredDocumentId, TenantId,
};
use diesel::{AsChangeset, Associations, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable, Clone, Associations)]
#[diesel(table_name = crate::schema::payment_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(belongs_to(crate::invoices::InvoiceRow, foreign_key=invoice_id))]
pub struct PaymentTransactionRow {
    pub id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub invoice_id: Option<InvoiceId>,
    pub provider_transaction_id: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub refunded_at: Option<NaiveDateTime>,
    pub amount: i64,
    pub currency: String,
    // TODO fees ?
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    // enum ?
    pub error_type: Option<String>,
    pub receipt_pdf_id: Option<StoredDocumentId>,
    pub checkout_session_id: Option<CheckoutSessionId>,
    pub pending_plan_version_id: Option<PlanVersionId>,
    /// Trigger-maintained analytics ingestion marker; never written by app code.
    pub store_committed_at: NaiveDateTime,
    /// Set when a customer initiated the payment (portal), so a settled invoice
    /// payment is attributed to them rather than System. Null for system auto-charges.
    pub initiated_by_customer_id: Option<CustomerId>,
    /// Wall-clock time the row was inserted. Distinct from `processed_at`,
    /// which only gets set on transition to a terminal state. Used by the
    /// reconciliation worker to filter Pending rows by age. TIMESTAMPTZ so the
    /// age math is unambiguous UTC.
    pub created_at: DateTime<Utc>,
    /// Customer action required to complete the charge (3DS/SCA). Present +
    /// status Pending = waiting on the customer. Serialized `PaymentNextAction`.
    pub next_action: Option<serde_json::Value>,
    /// Cumulative amount clawed back (partial refunds) while the row is still
    /// Settled; a full claw-back flips `status` to Refunded instead. The invoice
    /// nets this out of its settled-payments sum.
    pub amount_refunded: i64,
    /// Hosted in-flow-capture intent at the provider (Stancer — no webhooks),
    /// stored so the sweeper can recover a captured payment on a lost return.
    /// Write-once per row; cleared on supersede, close-out, or completion
    /// (invoice settled / checkout settled AND materialized).
    pub pending_provider_intent_id: Option<String>,
    pub pending_connection_id: Option<CustomerConnectionId>,
    /// Cumulative amount involuntarily clawed back (chargeback, recall, lost dispute).
    /// Distinct from `amount_refunded`, which is the voluntary refunded total; the invoice
    /// nets out this one only.
    pub amount_reversed: i64,
    pub reversal_kind: Option<ReversalKindEnum>,
    /// Raw provider cause or merchant free text behind `reversal_kind`.
    pub reversal_reason: Option<String>,
    /// Set on REFUND rows: the settled Payment row the money is given back from.
    pub parent_transaction_id: Option<PaymentTransactionId>,
    /// Set on REFUND rows backed by a finalized Refund credit note; NULL for an
    /// overpayment refund and for provider-dashboard refunds we only observed.
    pub credit_note_id: Option<CreditNoteId>,
    pub refund_mode: Option<RefundModeEnum>,
    /// Last time the reconcile sweep polled the provider for this row.
    pub last_reconciled_at: Option<DateTime<Utc>>,
    pub decline_kind: Option<DeclineKindEnum>,
    /// The part of `amount_reversed` lost to disputes: its invoice stays paid.
    pub amount_disputed: i64,
    /// What the provider withdrew above what the payment still held (refunded before): a loss
    /// to contest, never applied to the invoice or credit.
    pub amount_reversed_excess: i64,
    /// On a REFUND row without a credit note: the part of it that gave back money collected above
    /// the invoice when it was issued.
    pub refund_excess: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::payment_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentTransactionRowNew {
    pub id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub invoice_id: Option<InvoiceId>,
    pub provider_transaction_id: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    pub error_type: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub checkout_session_id: Option<CheckoutSessionId>,
    pub pending_plan_version_id: Option<PlanVersionId>,
    pub next_action: Option<serde_json::Value>,
    pub initiated_by_customer_id: Option<CustomerId>,
    pub pending_provider_intent_id: Option<String>,
    pub pending_connection_id: Option<CustomerConnectionId>,
    pub parent_transaction_id: Option<PaymentTransactionId>,
    pub credit_note_id: Option<CreditNoteId>,
    pub refund_mode: Option<RefundModeEnum>,
    pub reversal_reason: Option<String>,
}

#[derive(AsChangeset, Default)]
#[diesel(table_name = crate::schema::payment_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id))]
pub struct PaymentTransactionRowPatch {
    #[diesel(skip_insertion)]
    pub id: PaymentTransactionId,
    pub invoice_id: Option<Option<InvoiceId>>,
    pub status: Option<PaymentStatusEnum>,
    pub error_type: Option<Option<String>>,
    pub processed_at: Option<Option<NaiveDateTime>>,
    pub refunded_at: Option<Option<NaiveDateTime>>,
    pub provider_transaction_id: Option<Option<String>>,
    pub payment_method_id: Option<Option<CustomerPaymentMethodId>>,
    pub next_action: Option<Option<serde_json::Value>>,
    pub amount_refunded: Option<i64>,
    pub amount_reversed: Option<i64>,
    pub reversal_kind: Option<Option<ReversalKindEnum>>,
    pub reversal_reason: Option<Option<String>>,
    pub decline_kind: Option<Option<DeclineKindEnum>>,
    pub amount_disputed: Option<i64>,
    pub amount_reversed_excess: Option<i64>,
    pub refund_excess: Option<i64>,
    pub amount: Option<i64>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentTransactionWithMethodRow {
    #[diesel(embed)]
    pub transaction: PaymentTransactionRow,
    #[diesel(embed)]
    pub method: Option<CustomerPaymentMethodRow>,
}
