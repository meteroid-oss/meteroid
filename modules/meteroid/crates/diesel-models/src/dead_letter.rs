use chrono::NaiveDateTime;
use diesel::{Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

use crate::enums::DeadLetterStatusEnum;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::dead_letter_message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeadLetterMessageRow {
    pub id: Uuid,
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub dead_lettered_at: NaiveDateTime,
    pub last_error: Option<String>,
    pub status: DeadLetterStatusEnum,
    pub resolved_at: Option<NaiveDateTime>,
    pub resolved_by: Option<Uuid>,
    pub requeued_pgmq_msg_id: Option<i64>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::dead_letter_message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeadLetterMessageRowNew {
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Queryable, Selectable, Identifiable)]
#[diesel(table_name = crate::schema::dead_letter_alert_state)]
#[diesel(primary_key(queue))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeadLetterAlertStateRow {
    pub queue: String,
    pub last_alerted_at: NaiveDateTime,
}
