use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{ScheduledEventStatus, ScheduledEventTypeEnum};
use common_domain::ids::{SubscriptionId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, QueryableByName, Selectable};

#[derive(Debug, Clone, Queryable, QueryableByName, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::scheduled_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ScheduledEventRow {
    pub id: Uuid,
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub event_type: ScheduledEventTypeEnum,
    pub scheduled_time: NaiveDateTime,
    pub priority: i32,
    pub event_data: serde_json::Value,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub status: ScheduledEventStatus,
    pub retries: i32,
    pub last_retry_at: Option<NaiveDateTime>,
    pub error: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub source: String, // API, System, etc.
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::scheduled_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ScheduledEventRowNew {
    pub id: Uuid,
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub event_type: ScheduledEventTypeEnum,
    pub scheduled_time: NaiveDateTime,
    pub priority: i32,
    pub event_data: serde_json::Value,
    pub status: ScheduledEventStatus,
    pub retries: i32,
    pub source: String,
}
