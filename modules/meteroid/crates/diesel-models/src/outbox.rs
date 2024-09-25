use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::OutboxStatus;
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, QueryableByName, Selectable};
#[derive(Debug, Queryable, Identifiable, Selectable, QueryableByName)]
#[diesel(table_name = crate::schema::outbox)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OutboxRow {
    pub id: Uuid,
    pub event_type: String,
    pub tenant_id: Uuid,
    pub resource_id: Uuid,
    pub status: OutboxStatus,
    pub payload: Option<serde_json::Value>,
    pub created_at: NaiveDateTime,
    pub processing_started_at: Option<NaiveDateTime>,
    pub processing_completed_at: Option<NaiveDateTime>,
    pub processing_attempts: i32,
    pub error: Option<String>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::outbox)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OutboxRowNew {
    pub id: Uuid,
    pub event_type: String,
    pub resource_id: Uuid,
    pub tenant_id: Uuid,
    pub status: OutboxStatus,
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::outbox)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OutboxRowPatch {
    pub id: Uuid,
    pub status: OutboxStatus,
    pub processing_started_at: Option<NaiveDateTime>,
    pub processing_completed_at: Option<NaiveDateTime>,
    pub processing_attempts: i32,
    pub error: Option<String>,
}
