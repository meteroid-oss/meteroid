use crate::errors::IntoDbResult;
use crate::plan_version_options::{PlanVersionOptionRow, PlanVersionOptionRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{PlanOptionId, PlanVersionId, TenantId};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl PlanVersionOptionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanVersionOptionRow> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        let query = diesel::insert_into(o_dsl::plan_version_option).values(self);
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while inserting a plan option")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: &[PlanVersionOptionRowNew],
    ) -> DbResult<Vec<PlanVersionOptionRow>> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        if batch.is_empty() {
            return Ok(vec![]);
        }
        let query = diesel::insert_into(o_dsl::plan_version_option).values(batch);
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while inserting plan options")
            .into_db_result()
    }
}

impl PlanVersionOptionRow {
    /// The options of each version, by version then position.
    pub async fn list_by_plan_version_ids(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_version_ids: &[PlanVersionId],
    ) -> DbResult<Vec<PlanVersionOptionRow>> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        if plan_version_ids.is_empty() {
            return Ok(vec![]);
        }
        let query = o_dsl::plan_version_option
            .filter(o_dsl::tenant_id.eq(tenant_id))
            .filter(o_dsl::plan_version_id.eq_any(plan_version_ids))
            .order((o_dsl::plan_version_id, o_dsl::position, o_dsl::created_at))
            .select(PlanVersionOptionRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing plan options")
            .into_db_result()
    }

    pub async fn list_by_plan_version_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> DbResult<Vec<PlanVersionOptionRow>> {
        Self::list_by_plan_version_ids(conn, tenant_id, &[plan_version_id]).await
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PlanOptionId,
    ) -> DbResult<PlanVersionOptionRow> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        let query = o_dsl::plan_version_option
            .filter(o_dsl::tenant_id.eq(tenant_id))
            .filter(o_dsl::id.eq(id))
            .select(PlanVersionOptionRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .attach("Error while finding a plan option")
            .into_db_result()
    }

    pub async fn update_name_alias(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PlanOptionId,
        name: &str,
        alias: &str,
    ) -> DbResult<PlanVersionOptionRow> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        let query = diesel::update(o_dsl::plan_version_option)
            .filter(o_dsl::tenant_id.eq(tenant_id))
            .filter(o_dsl::id.eq(id))
            .set((o_dsl::name.eq(name), o_dsl::alias.eq(alias)));
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while updating a plan option")
            .into_db_result()
    }

    pub async fn set_hidden(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PlanOptionId,
        hidden: bool,
    ) -> DbResult<PlanVersionOptionRow> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        let query = diesel::update(o_dsl::plan_version_option)
            .filter(o_dsl::tenant_id.eq(tenant_id))
            .filter(o_dsl::id.eq(id))
            .set(o_dsl::hidden.eq(hidden));
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while hiding or showing a plan option")
            .into_db_result()
    }

    pub async fn set_position(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PlanOptionId,
        position: i32,
    ) -> DbResult<()> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        let query = diesel::update(o_dsl::plan_version_option)
            .filter(o_dsl::tenant_id.eq(tenant_id))
            .filter(o_dsl::id.eq(id))
            .set(o_dsl::position.eq(position));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while moving a plan option")
            .map(|_| ())
            .into_db_result()
    }

    /// Its links go with it.
    pub async fn delete(conn: &mut PgConn, tenant_id: TenantId, id: PlanOptionId) -> DbResult<()> {
        use crate::schema::plan_version_option::dsl as o_dsl;

        let query = diesel::delete(o_dsl::plan_version_option)
            .filter(o_dsl::tenant_id.eq(tenant_id))
            .filter(o_dsl::id.eq(id));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while removing a plan option")
            .map(|_| ())
            .into_db_result()
    }
}
