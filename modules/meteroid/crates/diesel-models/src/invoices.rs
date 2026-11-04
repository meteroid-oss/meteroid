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
}

/// An invoice's money: what it asks (`owed`: total less applied credit and reducing notes) and
/// what its settled collections did, net of refunds drawn on notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvoiceCollections {
    pub owed: i64,
    pub gross: i64,
    /// Clawed back as a failed collection: the invoice owes it again.
    pub failed: i64,
    /// Lost to disputes: the invoice stays as paid, the merchant bears the loss.
    pub disputed: i64,
    /// What refunds without a note gave back of the excess when each was issued: a reversal
    /// after them never counts it as collected again.
    pub excess_refunded: i64,
    /// What refunds without a note gave back beyond the excess (issued, in flight or settled):
    /// the invoice stays paid, but the credit it sold no longer stands on that money.
    pub refunded: i64,
    /// What finalized refund notes still owe back in cash, undrawn or in flight: out of hand for
    /// the credit the invoice sold, though it pays the invoice until it is refunded.
    pub owed_back: i64,
}

impl InvoiceCollections {
    /// What collections keep above what the invoice asks, before a refund without a note.
    pub fn excess(&self) -> i64 {
        (self.gross - self.failed - self.disputed - self.excess_refunded - self.owed).max(0)
    }

    pub fn amount_due(&self) -> i64 {
        (self.owed - (self.gross - self.failed - self.excess_refunded)).max(0)
    }

    /// The reversed or refunded money of the invoice that nothing collected again: what the
    /// credit it sold still stands on is missing by this much.
    pub fn uncollected(&self) -> i64 {
        let in_hand = self.gross
            - self.failed
            - self.disputed
            - self.excess_refunded
            - self.refunded
            - self.owed_back;
        (self.failed + self.disputed + self.refunded)
            .min(self.owed - in_hand)
            .max(0)
    }
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

#[cfg(test)]
mod tests {
    use super::InvoiceCollections;

    fn paid(owed: i64) -> InvoiceCollections {
        InvoiceCollections {
            owed,
            gross: owed,
            failed: 0,
            disputed: 0,
            excess_refunded: 0,
            refunded: 0,
            owed_back: 0,
        }
    }

    /// A 5000 purchase paid once: what refunds without a note, disputes and failures took is
    /// missing, together and at most what the invoice asks; money collected again backs it.
    #[test]
    fn refunded_money_is_missing_like_reversed_money() {
        assert_eq!(paid(5_000).uncollected(), 0);
        let refunded = InvoiceCollections {
            refunded: 2_000,
            ..paid(5_000)
        };
        assert_eq!(refunded.uncollected(), 2_000);
        assert_eq!(refunded.amount_due(), 0, "a refund keeps the invoice paid");
        let then_disputed = InvoiceCollections {
            disputed: 3_000,
            ..refunded
        };
        assert_eq!(then_disputed.uncollected(), 5_000);
        // The 1000 refunded out of a 1000 overpayment gave back excess only.
        let overpaid = InvoiceCollections {
            gross: 6_000,
            excess_refunded: 1_000,
            ..paid(5_000)
        };
        assert_eq!(overpaid.uncollected(), 0);
        let paid_again = InvoiceCollections {
            gross: 10_000,
            failed: 5_000,
            refunded: 1_000,
            ..paid(5_000)
        };
        assert_eq!(paid_again.uncollected(), 1_000);
        // 2000 refunded at the provider, then a note for the other 3000 not refunded yet: the
        // note lowers what is owed, its cash is already the customer's.
        let then_noted = InvoiceCollections {
            owed: 2_000,
            owed_back: 3_000,
            ..refunded
        };
        assert_eq!(then_noted.uncollected(), 2_000);
        assert_eq!(then_noted.amount_due(), 0);
    }
}
