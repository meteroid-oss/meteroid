use crate::api::shared::conversions::ProtoConv;
use meteroid_grpc::meteroid::admin::deadletter::v1 as proto;
use meteroid_store::domain::dead_letter::{DeadLetterMessage, DeadLetterQueueStats};
use meteroid_store::domain::enums::DeadLetterStatus;

pub fn to_proto_entry(
    m: &DeadLetterMessage,
    requeued_dead_letter_id: Option<String>,
) -> proto::DeadLetterEntry {
    proto::DeadLetterEntry {
        id: m.id.to_string(),
        tenant_id: m.tenant_id.map(|t| t.as_proto()),
        tenant_name: m.tenant_name.clone(),
        tenant_slug: m.tenant_slug.clone(),
        organization_id: m.organization_id.map(|o| o.as_proto()),
        organization_name: m.organization_name.clone(),
        organization_slug: m.organization_slug.clone(),
        queue: m.queue.clone(),
        pgmq_msg_id: m.pgmq_msg_id,
        message_json: m.message.as_ref().map(|v| v.to_string()),
        headers_json: m.headers.as_ref().map(|v| v.to_string()),
        read_count: m.read_ct,
        enqueued_at: m.enqueued_at.as_proto(),
        dead_lettered_at: m.dead_lettered_at.as_proto(),
        last_error: m.last_error.clone(),
        status: to_proto_status(&m.status).into(),
        resolved_at: m.resolved_at.map(|dt| dt.as_proto()),
        resolved_by: m.resolved_by.map(|u| u.to_string()),
        requeued_pgmq_msg_id: m.requeued_pgmq_msg_id,
        requeued_dead_letter_id,
    }
}

fn to_proto_status(s: &DeadLetterStatus) -> proto::DeadLetterStatus {
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

pub fn to_proto_org_item(
    o: meteroid_store::domain::dead_letter::OrganizationWithTenants,
) -> proto::OrganizationItem {
    proto::OrganizationItem {
        id: o.id.as_proto(),
        trade_name: o.trade_name,
        slug: o.slug,
        tenants: o
            .tenants
            .into_iter()
            .map(|t| proto::TenantItem {
                id: t.id.as_proto(),
                name: t.name,
                slug: t.slug,
            })
            .collect(),
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
