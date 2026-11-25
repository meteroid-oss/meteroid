use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{BaseId, ManagedQuantityId, SlotTransactionId, SubscriptionId};
use diesel_models::slot_transactions::SlotTransactionRow;
use o2o::o2o;

use crate::domain::SlotTransactionStatusEnum;
use crate::domain::subscription_components::SubscriptionFee;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
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
    pub managed_quantity_id: ManagedQuantityId,
    pub removal_waits_for_renewal: bool,
}

#[derive(Debug)]
pub struct SlotTransactionNewInternal {
    pub id: SlotTransactionId,
    /// The seats' unit: the code of the quantity they count when `quantity_id` is None (a fee
    /// recorded before quantities were named, I32).
    pub unit: String,
    /// The managed quantity the seats count.
    pub quantity_id: Option<ManagedQuantityId>,
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
                quantity_id,
                ..
            } => Some(Self {
                id: SlotTransactionId::new(),
                unit: unit.clone(),
                quantity_id: *quantity_id,
                delta: 0,
                prev_active_slots: *initial_slots as i32,
                effective_at: effective_date.and_time(NaiveTime::MIN),
                transaction_at: effective_date.and_time(NaiveTime::MIN),
            }),
            _ => None,
        }
    }
}

/// The charge a seat change is asked through: a plan charge of the subscription, or one of its
/// add-ons charging per seat. Charges counting one managed quantity share its count (R-137).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatCharge {
    Component(common_domain::ids::PriceComponentId),
    AddOn(common_domain::ids::AddOnId),
}

impl From<common_domain::ids::PriceComponentId> for SeatCharge {
    fn from(id: common_domain::ids::PriceComponentId) -> Self {
        SeatCharge::Component(id)
    }
}

impl From<common_domain::ids::AddOnId> for SeatCharge {
    fn from(id: common_domain::ids::AddOnId) -> Self {
        SeatCharge::AddOn(id)
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

/// The seats a change writes to: the quantity they count, its label, and the bounds every
/// charge of the subscription counting it sets (the strictest of each).
pub struct SlotForTransaction {
    pub unit: String,
    pub quantity_id: ManagedQuantityId,
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
    /// The taxed prorated invoice charged now for an increase, computed by the
    /// same path that the apply route bills. `None` for decreases (deferred, no
    /// immediate charge).
    pub adjustment_invoice: Option<ComputedInvoiceContent>,
    /// The MRR once the new count is held, in minor units; `None` when it could not be stated.
    pub mrr_after_cents: Option<i64>,
}
