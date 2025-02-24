use chrono::{NaiveDate, NaiveDateTime};
use uuid::Uuid;

use crate::enums::SubscriptionEventType;
use common_domain::ids::SubscriptionId;
use diesel::{Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Insertable, Selectable)]
#[diesel(table_name = crate::schema::subscription_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionEventRow {
    pub id: Uuid,
    pub mrr_delta: Option<i64>,
    pub event_type: SubscriptionEventType,
    pub created_at: NaiveDateTime,
    pub applies_to: NaiveDate,
    pub subscription_id: SubscriptionId,
    pub bi_mrr_movement_log_id: Option<Uuid>,
    pub details: Option<serde_json::Value>,
}
