use common_domain::ids::{EventId, TenantId};
use diesel::Insertable;

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::outbox_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OutboxEventRowNew {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub aggregate_id: String,
    pub aggregate_type: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}
