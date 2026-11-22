use crate::connector_sync_issues::ConnectorSyncIssueRow;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, ConnectorId, ConnectorSyncIssueId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl ConnectorSyncIssueRow {
    /// Records the latest failure of a resource, counting how often it failed.
    pub async fn record(
        conn: &mut PgConn,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        resource_type: &str,
        resource_id: Uuid,
        message: &str,
        seen_at: NaiveDateTime,
    ) -> DbResult<()> {
        diesel::sql_query(
            "INSERT INTO connector_sync_issue
                 (id, tenant_id, connector_id, resource_type, resource_id, message, occurrences,
                  first_seen_at, last_seen_at)
             SELECT $1, $2, c.id, $4, $5, $6, 1, $7, $7
             FROM connector c
             WHERE c.id = $3 AND c.tenant_id = $2
             ON CONFLICT (connector_id, resource_type, resource_id) DO UPDATE
             SET message = EXCLUDED.message,
                 occurrences = connector_sync_issue.occurrences + 1,
                 last_seen_at = EXCLUDED.last_seen_at",
        )
        .bind::<sql_types::Uuid, _>(ConnectorSyncIssueId::new())
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(connector_id)
        .bind::<sql_types::Text, _>(resource_type)
        .bind::<sql_types::Uuid, _>(resource_id)
        .bind::<sql_types::Text, _>(message)
        .bind::<sql_types::Timestamptz, _>(seen_at)
        .execute(conn)
        .await
        .map(|_| ())
        .attach("Error while recording a connector sync issue")
        .into_db_result()
    }

    pub async fn clear(
        conn: &mut PgConn,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        resource_type: &str,
        resource_id: Uuid,
    ) -> DbResult<()> {
        use crate::schema::connector_sync_issue::dsl as i;

        diesel::delete(i::connector_sync_issue)
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::connector_id.eq(connector_id))
            .filter(i::resource_type.eq(resource_type))
            .filter(i::resource_id.eq(resource_id))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while clearing a connector sync issue")
            .into_db_result()
    }

    /// Moves the open issues of the tenant's other connectors of the same provider to
    /// `connector_id`, so a reconnected integration still lists what never synced.
    pub async fn adopt_from_previous_connectors(
        conn: &mut PgConn,
        tenant_id: TenantId,
        connector_id: ConnectorId,
    ) -> DbResult<()> {
        diesel::sql_query(
            "WITH moved AS (
                 DELETE FROM connector_sync_issue i
                 USING connector previous, connector current
                 WHERE current.id = $2 AND current.tenant_id = $1
                   AND previous.tenant_id = $1 AND previous.provider = current.provider
                   AND previous.id <> current.id
                   AND i.connector_id = previous.id
                 RETURNING i.*
             )
             INSERT INTO connector_sync_issue
                 (id, tenant_id, connector_id, resource_type, resource_id, message, occurrences,
                  first_seen_at, last_seen_at)
             SELECT id, tenant_id, $2, resource_type, resource_id, message, occurrences,
                    first_seen_at, last_seen_at
             FROM moved
             ON CONFLICT (connector_id, resource_type, resource_id) DO NOTHING",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(connector_id)
        .execute(conn)
        .await
        .map(|_| ())
        .attach("Error while moving connector sync issues to a reconnected connector")
        .into_db_result()
    }

    /// Most recent first, all of them when `limit` is `None`.
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        limit: Option<i64>,
    ) -> DbResult<Vec<ConnectorSyncIssueRow>> {
        use crate::schema::connector_sync_issue::dsl as i;

        let mut query = i::connector_sync_issue
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::connector_id.eq(connector_id))
            .order(i::last_seen_at.desc())
            .into_boxed();
        if let Some(limit) = limit {
            query = query.limit(limit);
        }
        query
            .get_results(conn)
            .await
            .attach("Error while listing connector sync issues")
            .into_db_result()
    }
}
