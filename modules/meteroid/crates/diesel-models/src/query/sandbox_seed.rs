use crate::enums::SandboxSeedStatusEnum;
use crate::errors::IntoDbResult;
use crate::sandbox_seed::SandboxSeedRow;
use crate::{DbResult, PgConn};
use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl SandboxSeedRow {
    /// Marks the seed of `tenant_id` started; a re-run starts it over.
    pub async fn start(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<()> {
        use crate::schema::sandbox_seed::dsl as s;

        let query = diesel::insert_into(s::sandbox_seed)
            .values((
                s::tenant_id.eq(tenant_id),
                s::status.eq(SandboxSeedStatusEnum::InProgress),
            ))
            .on_conflict(s::tenant_id)
            .do_update()
            .set((
                s::status.eq(SandboxSeedStatusEnum::InProgress),
                s::failure_reason.eq(None::<String>),
                s::started_at.eq(common_domain::clock::wall().naive_utc()),
                s::finished_at.eq(None::<chrono::NaiveDateTime>),
            ));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while starting a sandbox seed")
            .into_db_result()
            .map(|_| ())
    }

    /// Closes the running seed of `tenant_id`: completed, or failed with a reason. False when no
    /// seed of it was running.
    pub async fn finish(
        conn: &mut PgConn,
        tenant_id: TenantId,
        failure_reason: Option<String>,
    ) -> DbResult<bool> {
        use crate::schema::sandbox_seed::dsl as s;

        let status = if failure_reason.is_some() {
            SandboxSeedStatusEnum::Failed
        } else {
            SandboxSeedStatusEnum::Completed
        };
        let query = diesel::update(s::sandbox_seed)
            .filter(s::tenant_id.eq(tenant_id))
            .filter(s::status.eq(SandboxSeedStatusEnum::InProgress))
            .set((
                s::status.eq(status),
                s::failure_reason.eq(failure_reason),
                s::finished_at.eq(common_domain::clock::wall().naive_utc()),
            ));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while finishing a sandbox seed")
            .into_db_result()
            .map(|updated| updated > 0)
    }

    /// Fails every seed still running an hour after it started, far longer than any seed takes:
    /// the process running it is gone. Returns their tenants.
    pub async fn fail_interrupted(
        conn: &mut PgConn,
        failure_reason: &str,
    ) -> DbResult<Vec<TenantId>> {
        use crate::schema::sandbox_seed::dsl as s;

        let query = diesel::update(s::sandbox_seed)
            .filter(s::status.eq(SandboxSeedStatusEnum::InProgress))
            .filter(
                s::started_at
                    .lt(common_domain::clock::wall().naive_utc() - chrono::Duration::hours(1)),
            )
            .set((
                s::status.eq(SandboxSeedStatusEnum::Failed),
                s::failure_reason.eq(failure_reason),
                s::finished_at.eq(common_domain::clock::wall().naive_utc()),
            ))
            .returning(s::tenant_id);
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while failing interrupted sandbox seeds")
            .into_db_result()
    }

    /// The tenants among `tenant_ids` whose seed is still running.
    pub async fn seeding_among(
        conn: &mut PgConn,
        tenant_ids: &[TenantId],
    ) -> DbResult<Vec<TenantId>> {
        use crate::schema::sandbox_seed::dsl as s;

        if tenant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = s::sandbox_seed
            .filter(s::tenant_id.eq_any(tenant_ids))
            .filter(s::status.eq(SandboxSeedStatusEnum::InProgress))
            .select(s::tenant_id);
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing seeding sandboxes")
            .into_db_result()
    }

    pub async fn find(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<Option<SandboxSeedRow>> {
        use crate::schema::sandbox_seed::dsl as s;

        let query = s::sandbox_seed
            .filter(s::tenant_id.eq(tenant_id))
            .select(SandboxSeedRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding a sandbox seed")
            .into_db_result()
    }
}

/// A seeded Sandbox's signatures dated before what they started: a subscription signed a week
/// before it started, a pending cancellation given twelve days' notice, never later than now.
pub async fn backdate_sandbox_signatures(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<()> {
    use diesel::sql_types;

    diesel::sql_query(
        r#"
        UPDATE subscription
        SET created_at = LEAST(created_at, (COALESCE(billing_start_date, start_date) - 7)::timestamp)
        WHERE tenant_id = $1
        "#,
    )
    .bind::<sql_types::Uuid, _>(tenant_id)
    .execute(conn)
    .await
    .attach("Error while backdating sandbox subscriptions")
    .into_db_result()?;
    diesel::sql_query(
        r#"
        UPDATE scheduled_event
        SET created_at = LEAST(created_at, scheduled_time - interval '12 days')
        WHERE tenant_id = $1 AND event_type = 'CANCEL_SUBSCRIPTION' AND status = 'PENDING'
        "#,
    )
    .bind::<sql_types::Uuid, _>(tenant_id)
    .execute(conn)
    .await
    .attach("Error while backdating sandbox cancellations")
    .into_db_result()
    .map(|_| ())
}
