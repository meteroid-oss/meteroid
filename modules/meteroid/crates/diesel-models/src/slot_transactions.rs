use chrono::NaiveDateTime;
use uuid::Uuid;

use common_domain::ids::{PriceComponentId, SubscriptionId};
use diesel::{Insertable, Queryable};

#[derive(Queryable, Debug, Insertable)]
#[diesel(table_name = crate::schema::slot_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SlotTransactionRow {
    pub id: Uuid,
    pub price_component_id: PriceComponentId,
    pub subscription_id: SubscriptionId,
    pub delta: i32,
    pub prev_active_slots: i32,
    pub effective_at: NaiveDateTime,
    pub transaction_at: NaiveDateTime,
}
