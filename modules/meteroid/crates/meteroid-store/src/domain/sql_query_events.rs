//! Outbox payload of a scheduled SQL query run (EE intelligence module), success or failure.
//! The id derives from the schedule and its due time, so a due run announces itself once.
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{
    BaseId, EventId, SqlQueryId, SqlQueryRunId, SqlQueryScheduleId, TenantId,
};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

const EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x6c1d_2a8e_93f4_4b57_a0e2_5d8c_17f9_b346);

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlScheduledRunEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub schedule_id: SqlQueryScheduleId,
    pub saved_query_id: SqlQueryId,
    pub query_name: String,
    /// None when the run was refused before it started (a plan limit, an invalid query).
    pub run_id: Option<SqlQueryRunId>,
    pub succeeded: bool,
    pub error: Option<String>,
    pub row_count: Option<u64>,
    pub truncated: bool,
    /// The period `[start, end)` the run reports on, bound to its `start`/`end` parameters.
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    pub ran_at: DateTime<Utc>,
}

impl SqlScheduledRunEvent {
    pub const ROUTING_KEY: &'static str = "sql.scheduled_run.completed";

    pub fn event_id(schedule_id: SqlQueryScheduleId, due_at: DateTime<Utc>) -> EventId {
        let mut key = schedule_id.as_uuid().as_bytes().to_vec();
        key.extend_from_slice(&due_at.timestamp_millis().to_be_bytes());
        EventId::from(Uuid::new_v5(&EVENT_NAMESPACE, &key))
    }
}
