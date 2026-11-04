use crate::errors::IntoDbResult;
use crate::instance_telemetry::{InstanceTelemetryRow, InstanceUsageRow};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl InstanceTelemetryRow {
    pub async fn get(conn: &mut PgConn) -> DbResult<Option<Self>> {
        use crate::schema::instance_telemetry::dsl;

        dsl::instance_telemetry
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching the instance telemetry identity")
            .into_db_result()
    }

    /// Creates the identity unless one exists, and returns the stored one.
    pub async fn get_or_create(
        conn: &mut PgConn,
        instance_id: uuid::Uuid,
        version: &str,
    ) -> DbResult<Self> {
        use crate::schema::instance_telemetry::dsl;

        diesel::insert_into(dsl::instance_telemetry)
            .values((
                dsl::instance_id.eq(instance_id),
                dsl::first_version.eq(version),
            ))
            .on_conflict(dsl::id)
            .do_nothing()
            .execute(conn)
            .await
            .attach("Error while creating the instance telemetry identity")
            .into_db_result()?;

        dsl::instance_telemetry
            .select(Self::as_select())
            .first(conn)
            .await
            .attach("Error while fetching the instance telemetry identity")
            .into_db_result()
    }

    pub async fn record_sent(conn: &mut PgConn, at: NaiveDateTime) -> DbResult<()> {
        use crate::schema::instance_telemetry::dsl;

        diesel::update(dsl::instance_telemetry)
            .set(dsl::last_sent_at.eq(at))
            .execute(conn)
            .await
            .attach("Error while recording the telemetry report")
            .into_db_result()?;
        Ok(())
    }
}

impl InstanceUsageRow {
    pub async fn query(conn: &mut PgConn) -> DbResult<Self> {
        diesel::sql_query(
            "WITH live AS ( \
               SELECT id FROM tenant WHERE environment = 'PRODUCTION' AND archived_at IS NULL \
             ) \
             SELECT \
               (SELECT COUNT(*) FROM organization WHERE archived_at IS NULL) AS organizations, \
               (SELECT COUNT(*) FROM live) AS tenants_live, \
               (SELECT COUNT(*) FROM tenant WHERE environment <> 'PRODUCTION' AND archived_at IS NULL) \
                 AS tenants_other, \
               (SELECT COUNT(*) FROM \"user\" WHERE archived_at IS NULL) AS users, \
               (SELECT COUNT(*) FROM customer WHERE archived_at IS NULL \
                  AND tenant_id IN (SELECT id FROM live)) AS customers, \
               (SELECT COUNT(*) FROM subscription WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE') \
                  AND tenant_id IN (SELECT id FROM live)) AS subscriptions_active, \
               (SELECT COUNT(*) FROM plan WHERE status = 'ACTIVE' AND archived_at IS NULL \
                  AND tenant_id IN (SELECT id FROM live)) AS plans_published, \
               (SELECT COUNT(*) FROM billable_metric WHERE archived_at IS NULL \
                  AND tenant_id IN (SELECT id FROM live)) AS billable_metrics, \
               (SELECT COUNT(*) FROM invoice WHERE finalized_at >= now() - interval '30 days' \
                  AND tenant_id IN (SELECT id FROM live)) AS invoices_finalized_30d, \
               ARRAY(SELECT DISTINCT currency FROM subscription \
                  WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE') AND tenant_id IN (SELECT id FROM live) \
                  ORDER BY 1) AS currencies, \
               ARRAY(SELECT DISTINCT lower(provider::text) FROM connector \
                  WHERE status = 'ACTIVE' AND provider <> 'MOCK' \
                  AND tenant_id IN (SELECT id FROM live) ORDER BY 1) AS integrations, \
               (current_setting('server_version_num')::int / 10000) AS postgres_major",
        )
        .get_result(conn)
        .await
        .attach("Error while counting instance usage")
        .into_db_result()
    }
}
