use chrono::NaiveDateTime;
use common_domain::ids::{ConnectorId, ConnectorSyncIssueId, TenantId};
use diesel::{Identifiable, Queryable, QueryableByName, Selectable};
use uuid::Uuid;

#[derive(Debug, Clone, Identifiable, Queryable, QueryableByName, Selectable)]
#[diesel(table_name = crate::schema::connector_sync_issue)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ConnectorSyncIssueRow {
    pub id: ConnectorSyncIssueId,
    pub tenant_id: TenantId,
    pub connector_id: ConnectorId,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub message: String,
    pub occurrences: i32,
    pub first_seen_at: NaiveDateTime,
    pub last_seen_at: NaiveDateTime,
}
