use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{BaseId, SlotTransactionId, SubscriptionId};
use diesel_models::slot_transactions::SlotTransactionRow;
use o2o::o2o;

use crate::domain::SlotTransactionStatusEnum;
use crate::domain::subscription_components::SubscriptionFee;
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

impl SlotTransactionNewInternal {
    /// Build a seed slot transaction from a Slot fee, or None for non-Slot fees.
    pub fn from_fee(fee: &SubscriptionFee, effective_date: NaiveDate) -> Option<Self> {
        match fee {
            SubscriptionFee::Slot {
                initial_slots,
                unit,
                ..
            } => Some(Self {
                id: SlotTransactionId::new(),
                unit: unit.clone(),
                delta: 0,
                prev_active_slots: *initial_slots as i32,
                effective_at: effective_date.and_time(NaiveTime::MIN),
                transaction_at: effective_date.and_time(NaiveTime::MIN),
            }),
            _ => None,
        }
    }

    /// Convert into a row ready for insertion.
    pub fn into_row(self, subscription_id: SubscriptionId) -> SlotTransactionRow {
        SlotTransactionRow {
            id: self.id,
            subscription_id,
            delta: self.delta,
            prev_active_slots: self.prev_active_slots,
            effective_at: self.effective_at,
            transaction_at: self.transaction_at,
            unit: self.unit,
            status: diesel_models::enums::SlotTransactionStatusEnum::Active,
            invoice_id: None,
        }
    }
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

#[derive(Debug)]
pub struct SlotUpdatePreview {
    pub current_slots: i32,
    pub new_slots: i32,
    pub delta: i32,
    pub unit: String,
    pub unit_rate: Decimal,
    pub prorated_amount: Decimal,
    pub full_period_amount: Decimal,
    pub days_remaining: i32,
    pub days_total: i32,
    pub effective_at: chrono::NaiveDate,
    pub current_period_end: chrono::NaiveDate,
    pub next_invoice_delta: Decimal,
}
