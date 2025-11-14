use chrono::NaiveDateTime;
use common_domain::ids::{SlotTransactionId, SubscriptionId};
use diesel_models::slot_transactions::SlotTransactionRow;
use o2o::o2o;

use crate::domain::SlotTransactionStatusEnum;
use common_domain::ids::InvoiceId;
use rust_decimal::Decimal;

#[derive(Clone, Debug, o2o)]
#[from_owned(SlotTransactionRow)]
#[owned_into(SlotTransactionRow)]
pub struct SlotTransaction {
    pub id: SlotTransactionId,
    pub subscription_id: SubscriptionId,
    pub delta: i32,
    pub prev_active_slots: i32,
    pub effective_at: NaiveDateTime,
    pub transaction_at: NaiveDateTime,
    pub unit: String,
    #[from(~.into())]
    #[into(~.into())]
    pub status: SlotTransactionStatusEnum,
    pub invoice_id: Option<InvoiceId>,
}

pub struct SlotTransactionNewInternal {
    pub id: SlotTransactionId,
    // TODO product will be more pertinent (addon & sub compo)
    pub unit: String,
    pub delta: i32,
    pub prev_active_slots: i32,
    pub effective_at: NaiveDateTime,
    pub transaction_at: NaiveDateTime,
}

/// Billing mode for slot upgrades (downgrades are always deferred)
#[derive(Debug, Clone, Copy)]
pub enum SlotUpgradeBillingMode {
    /// Standard self-serve / checkout flow.
    /// Nothing is created until payment is completed via checkout page
    OnCheckout,

    /// - Invoice created (respects auto_advance/charge_automatically)
    /// - Pending slot transaction linked to invoice
    /// - Slots activate automatically via orchestration when invoice is paid
    OnInvoicePaid,

    /// - Slots activated immediately (active transaction created)
    /// - Invoice created (respects auto_advance/charge_automatically)
    Optimistic,
    // TODO add deferred / EndOfPeriod
}

#[derive(Debug)]
pub struct UpdateSlotsResult {
    pub new_slot_count: i32,
    pub delta_applied: i32,
    pub invoice_id: Option<InvoiceId>,
    pub prorated_amount: Option<Decimal>,
    /// false if waiting payment
    pub slots_active: bool,
}

pub struct SlotForTransaction {
    pub unit: String,
    pub unit_rate: Decimal,
    pub min_slots: Option<u32>,
    pub max_slots: Option<u32>,
}
