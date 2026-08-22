use crate::customer_payment_methods::CustomerPaymentMethodRow;
use crate::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{
    CheckoutSessionId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, InvoiceId,
    PaymentTransactionId, PlanVersionId, StoredDocumentId, TenantId,
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
    /// Set when a customer initiated the payment (portal), so a settled invoice
    /// payment is attributed to them rather than System. Null for system auto-charges.
    pub initiated_by_customer_id: Option<CustomerId>,
    /// Hosted in-flow-capture intent at the provider (Stancer — no webhooks),
    /// stored so the sweeper can recover a captured payment on a lost return.
    /// Write-once per row; cleared on supersede, close-out, or completion
    /// (invoice settled / checkout settled AND materialized).
    pub pending_provider_intent_id: Option<String>,
    pub pending_connection_id: Option<CustomerConnectionId>,
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
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentTransactionWithMethodRow {
    #[diesel(embed)]
    pub transaction: PaymentTransactionRow,
    #[diesel(embed)]
    pub method: Option<CustomerPaymentMethodRow>,
}
