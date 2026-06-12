use chrono::NaiveDateTime;
use common_domain::ids::{EntityActivityId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

use crate::enums::ActorTypeEnum;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::entity_activity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntityActivityRow {
    pub id: EntityActivityId,
    pub tenant_id: TenantId,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub activity_type: String,
    pub actor_type: ActorTypeEnum,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub occurred_at: NaiveDateTime,
    pub agg_customer_id: Option<Uuid>,
    pub agg_subscription_id: Option<Uuid>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::entity_activity)]
pub struct EntityActivityRowNew {
    pub id: EntityActivityId,
    pub tenant_id: TenantId,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub activity_type: String,
    pub actor_type: ActorTypeEnum,
    pub actor_uuid: Option<Uuid>,
    pub actor_alias: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub agg_customer_id: Option<Uuid>,
    pub agg_subscription_id: Option<Uuid>,
}
