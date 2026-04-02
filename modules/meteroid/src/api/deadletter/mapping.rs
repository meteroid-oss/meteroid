use crate::api::shared::conversions::ProtoConv;
use meteroid_grpc::meteroid::admin::deadletter::v1 as proto;
use meteroid_store::domain::dead_letter::{
    DeadLetterMessage, DeadLetterQueueStats, DeadLetterStatus,
};

pub fn to_proto_entry(m: DeadLetterMessage) -> proto::DeadLetterEntry {
    proto::DeadLetterEntry {
        id: m.id.to_string(),
        queue: m.queue,
        pgmq_msg_id: m.pgmq_msg_id,
        message_json: m.message.map(|v| v.to_string()),
        headers_json: m.headers.map(|v| v.to_string()),
        read_count: m.read_ct,
        enqueued_at: m.enqueued_at.as_proto(),
        dead_lettered_at: m.dead_lettered_at.as_proto(),
        last_error: m.last_error,
        status: to_proto_status(m.status).into(),
        resolved_at: m.resolved_at.map(|dt| dt.as_proto()),
        resolved_by: m.resolved_by.map(|u| u.to_string()),
        requeued_pgmq_msg_id: m.requeued_pgmq_msg_id,
    }
}

pub fn to_proto_status(s: DeadLetterStatus) -> proto::DeadLetterStatus {
    match s {
        DeadLetterStatus::Pending => proto::DeadLetterStatus::Pending,
        DeadLetterStatus::Requeued => proto::DeadLetterStatus::Requeued,
        DeadLetterStatus::Discarded => proto::DeadLetterStatus::Discarded,
    }
}

pub fn from_proto_status(s: proto::DeadLetterStatus) -> Option<DeadLetterStatus> {
    match s {
        proto::DeadLetterStatus::Unspecified => None,
        proto::DeadLetterStatus::Pending => Some(DeadLetterStatus::Pending),
        proto::DeadLetterStatus::Requeued => Some(DeadLetterStatus::Requeued),
        proto::DeadLetterStatus::Discarded => Some(DeadLetterStatus::Discarded),
    }
}

pub fn to_proto_queue_health(s: DeadLetterQueueStats) -> proto::QueueHealth {
    proto::QueueHealth {
        queue: s.queue,
        pending_count: s.pending_count,
        requeued_count: s.requeued_count,
        discarded_count: s.discarded_count,
    }
}
