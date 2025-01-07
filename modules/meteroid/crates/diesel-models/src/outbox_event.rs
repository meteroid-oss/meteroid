use uuid::Uuid;

use diesel::Insertable;

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::outbox_event)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OutboxEventRowNew {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub aggregate_id: String,
    pub aggregate_type: String,
    pub event_type: String,
    pub payload: Option<serde_json::Value>,
    pub local_id: String,
}
