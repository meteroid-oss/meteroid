


use chrono::NaiveDateTime;
use uuid::Uuid;


use diesel::{Identifiable, Queryable};



#[derive(Queryable, Debug)]
#[diesel(table_name = crate::schema::slot_transaction)]
pub struct SlotTransaction {
    pub id: Uuid,
    pub price_component_id: Uuid,
    pub subscription_id: Uuid,
    pub delta: i32,
    pub prev_active_slots: i32,
    pub effective_at: NaiveDateTime,
    pub transaction_at: NaiveDateTime,
}