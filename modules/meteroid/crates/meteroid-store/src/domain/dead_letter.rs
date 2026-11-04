use chrono::NaiveDateTime;
use common_domain::ids::TenantId;

pub struct DeadLetterMessageNew {
    pub tenant_id: Option<TenantId>,
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub last_error: Option<String>,
}

impl From<DeadLetterMessageNew> for diesel_models::dead_letter::DeadLetterMessageRowNew {
    fn from(value: DeadLetterMessageNew) -> Self {
        Self {
            tenant_id: value.tenant_id,
            queue: value.queue,
            pgmq_msg_id: value.pgmq_msg_id,
            message: value.message,
            headers: value.headers,
            read_ct: value.read_ct,
            enqueued_at: value.enqueued_at,
            last_error: value.last_error,
        }
    }
}
