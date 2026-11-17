use chrono::NaiveDateTime;

use crate::enums::{ActorTypeEnum, SubscriptionChangeKindEnum};
use common_domain::ids::{ScheduledEventId, SubscriptionId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

/// A plan change or amendment scheduled for a later date: its request, and the rows it wrote as
/// facts, which withdrawing it removes. It is applied once a Switch row names it.
#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::subscription_change)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionChangeRow {
    pub id: ScheduledEventId,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub kind: SubscriptionChangeKindEnum,
    pub effective_at: NaiveDateTime,
    pub change: serde_json::Value,
    pub written_facts: Option<serde_json::Value>,
    pub withdrawn_at: Option<NaiveDateTime>,
    pub withdrawn_reason: Option<String>,
    pub source: String,
    pub created_by_customer: bool,
    pub actor_type: ActorTypeEnum,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::subscription_change)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SubscriptionChangeRowNew {
    pub id: ScheduledEventId,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub kind: SubscriptionChangeKindEnum,
    pub effective_at: NaiveDateTime,
    pub change: serde_json::Value,
    pub written_facts: Option<serde_json::Value>,
    pub source: String,
    pub created_by_customer: bool,
    pub actor_type: ActorTypeEnum,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
