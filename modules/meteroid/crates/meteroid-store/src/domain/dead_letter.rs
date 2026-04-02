use chrono::NaiveDateTime;
use common_domain::ids::{OrganizationId, TenantId};
use diesel_models::dead_letter::{DeadLetterMessageRow, DeadLetterWithTenantRow};
use o2o::o2o;
use uuid::Uuid;

use crate::domain::enums::DeadLetterStatus;

#[derive(Debug, Clone, o2o)]
#[from_owned(DeadLetterMessageRow)]
pub struct DeadLetterMessage {
    pub id: Uuid,
    pub tenant_id: Option<TenantId>,
    #[ghost({None})]
    pub tenant_name: Option<String>,
    #[ghost({None})]
    pub tenant_slug: Option<String>,
    #[ghost({None})]
    pub organization_id: Option<OrganizationId>,
    #[ghost({None})]
    pub organization_name: Option<String>,
    #[ghost({None})]
    pub organization_slug: Option<String>,
    pub queue: String,
    pub pgmq_msg_id: i64,
    pub message: Option<serde_json::Value>,
    pub headers: Option<serde_json::Value>,
    pub read_ct: i32,
    pub enqueued_at: NaiveDateTime,
    pub dead_lettered_at: NaiveDateTime,
    pub last_error: Option<String>,
    #[from(~.into())]
    pub status: DeadLetterStatus,
    pub resolved_at: Option<NaiveDateTime>,
    pub resolved_by: Option<Uuid>,
    pub requeued_pgmq_msg_id: Option<i64>,
    pub created_at: NaiveDateTime,
}

impl From<DeadLetterWithTenantRow> for DeadLetterMessage {
    fn from(r: DeadLetterWithTenantRow) -> Self {
        let mut msg: DeadLetterMessage = r.dead_letter.into();
        msg.tenant_name = r.tenant_name;
        msg.tenant_slug = r.tenant_slug;
        msg.organization_id = r.organization_id;
        msg.organization_name = r.organization_name;
        msg.organization_slug = r.organization_slug;
        msg
    }
}

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

#[derive(Debug, Clone)]
pub struct DeadLetterQueueStats {
    pub queue: String,
    pub pending_count: i64,
    pub requeued_count: i64,
    pub discarded_count: i64,
}

#[derive(Debug, Clone)]
pub struct OrganizationWithTenants {
    pub id: OrganizationId,
    pub trade_name: String,
    pub slug: String,
    pub tenants: Vec<TenantSummary>,
}

#[derive(Debug, Clone)]
pub struct TenantSummary {
    pub id: TenantId,
    pub name: String,
    pub slug: String,
}
