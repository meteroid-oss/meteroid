use chrono::NaiveDateTime;

use common_domain::ids::{SlotTransactionId, SubscriptionId};
use diesel::{Insertable, Queryable};

#[derive(Queryable, Debug, Insertable)]
#[diesel(table_name = crate::schema::slot_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SlotTransactionRow {
    pub id: SlotTransactionId,
    pub subscription_id: SubscriptionId,
    pub delta: i32,
    pub prev_active_slots: i32,
    pub effective_at: NaiveDateTime,
    pub transaction_at: NaiveDateTime,
    pub unit: String,
}
