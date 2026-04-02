use chrono::NaiveDateTime;
use diesel_models::dead_letter::DeadLetterMessageRow;
use o2o::o2o;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadLetterStatus {
    Pending,
    Requeued,
    Discarded,
}

impl From<diesel_models::enums::DeadLetterStatusEnum> for DeadLetterStatus {
    fn from(value: diesel_models::enums::DeadLetterStatusEnum) -> Self {
        match value {
            diesel_models::enums::DeadLetterStatusEnum::Pending => DeadLetterStatus::Pending,
            diesel_models::enums::DeadLetterStatusEnum::Requeued => DeadLetterStatus::Requeued,
            diesel_models::enums::DeadLetterStatusEnum::Discarded => DeadLetterStatus::Discarded,
        }
    }
}

impl From<DeadLetterStatus> for diesel_models::enums::DeadLetterStatusEnum {
    fn from(value: DeadLetterStatus) -> Self {
        match value {
            DeadLetterStatus::Pending => diesel_models::enums::DeadLetterStatusEnum::Pending,
            DeadLetterStatus::Requeued => diesel_models::enums::DeadLetterStatusEnum::Requeued,
            DeadLetterStatus::Discarded => diesel_models::enums::DeadLetterStatusEnum::Discarded,
        }
    }
}

#[derive(Debug, Clone, o2o)]
#[from_owned(DeadLetterMessageRow)]
#[owned_into(DeadLetterMessageRow)]
pub struct DeadLetterMessage {
    pub id: Uuid,
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub dead_lettered_at: NaiveDateTime,
    pub last_error: Option<String>,
    #[from(~.into())]
    #[into(~.into())]
    pub status: DeadLetterStatus,
    pub resolved_at: Option<NaiveDateTime>,
    pub resolved_by: Option<Uuid>,
    pub requeued_pgmq_msg_id: Option<i64>,
    pub created_at: NaiveDateTime,
}

pub struct DeadLetterMessageNew {
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

#[derive(Debug, Clone)]
pub struct DeadLetterQueueStats {
    pub queue: String,
    pub pending_count: i64,
    pub requeued_count: i64,
    pub discarded_count: i64,
}
