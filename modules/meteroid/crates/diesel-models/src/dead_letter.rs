use chrono::NaiveDateTime;
use common_domain::ids::TenantId;
use diesel::Insertable;

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::dead_letter_message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeadLetterMessageRowNew {
    pub tenant_id: Option<TenantId>,
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub last_error: Option<String>,
}
