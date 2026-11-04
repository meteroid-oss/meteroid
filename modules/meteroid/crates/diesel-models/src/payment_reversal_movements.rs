use crate::enums::ReversalKindEnum;
use chrono::NaiveDateTime;
use common_domain::ids::{CustomerId, InvoiceId, PaymentTransactionId, TenantId};
use diesel::{Insertable, Queryable, Selectable};
use uuid::Uuid;

/// One applied reversal (positive) or reinstatement (negative) of a payment.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = crate::schema::payment_reversal_movement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentReversalMovementRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub payment_transaction_id: PaymentTransactionId,
    pub invoice_id: Option<InvoiceId>,
    pub customer_id: CustomerId,
    /// Lost to a dispute (the invoice stays paid), or a failed collection (it reopens).
    pub disputed: bool,
    /// Signed change of the payment's applied `amount_reversed`.
    pub amount: i64,
    /// Signed change of what the provider took above what the payment held.
    pub excess: i64,
    pub reversal_kind: Option<ReversalKindEnum>,
    pub provider_ref: Option<String>,
    pub occurred_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::payment_reversal_movement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentReversalMovementRowNew {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub payment_transaction_id: PaymentTransactionId,
    pub invoice_id: Option<InvoiceId>,
    pub customer_id: CustomerId,
    pub disputed: bool,
    pub amount: i64,
    pub excess: i64,
    pub reversal_kind: Option<ReversalKindEnum>,
    pub provider_ref: Option<String>,
    pub occurred_at: NaiveDateTime,
}
