//! Outbox payloads of a workflow run starting and ending (EE workflows module). The id derives
//! from the run and the transition, so a run announces each once.
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, EventId, TenantId, WorkflowId, WorkflowRunId};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

const EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x7c41_0e9b_d2a3_4f58_9b06_3e8a_5d17_c4f2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowRunTransition {
    Started,
    Succeeded,
    Failed,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub run_id: WorkflowRunId,
    pub workflow_id: WorkflowId,
    pub transition: WorkflowRunTransition,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub failed_step: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl WorkflowRunEvent {
    pub const STARTED_ROUTING_KEY: &'static str = "workflow_run.started";
    pub const SUCCEEDED_ROUTING_KEY: &'static str = "workflow_run.succeeded";
    pub const FAILED_ROUTING_KEY: &'static str = "workflow_run.failed";

    pub fn event_id(run_id: WorkflowRunId, transition: WorkflowRunTransition) -> EventId {
        let mut name = run_id.as_uuid().as_bytes().to_vec();
        name.push(transition as u8);
        EventId::from(Uuid::new_v5(&EVENT_NAMESPACE, &name))
    }
}
