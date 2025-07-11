use chrono::NaiveDateTime;
use common_domain::ids::{SlotTransactionId, SubscriptionId};
use diesel_models::slot_transactions::SlotTransactionRow;
use o2o::o2o;

#[derive(Clone, Debug, o2o)]
#[map_owned(SlotTransactionRow)]
pub struct SlotTransaction {
    pub id: SlotTransactionId,
    pub unit: String,
    pub subscription_id: SubscriptionId,
    pub delta: i32,
    pub prev_active_slots: i32,
    pub effective_at: NaiveDateTime,
    pub transaction_at: NaiveDateTime,
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
