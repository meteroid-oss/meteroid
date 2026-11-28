use crate::errors::IntoDbResult;
use crate::tenant_modules::TenantModuleRow;
use crate::{DbResult, PgConn};
use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl TenantModuleRow {
    pub async fn list(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<Vec<TenantModuleRow>> {
        use crate::schema::tenant_module::dsl as m;

        let query = m::tenant_module
            .filter(m::tenant_id.eq(tenant_id))
            .order(m::module.asc())
            .select(TenantModuleRow::as_select());
        crate::debug_sql(&query);
        query
            .load(conn)
            .await
            .attach("Error while listing tenant modules")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        module: &str,
    ) -> DbResult<Option<TenantModuleRow>> {
        use crate::schema::tenant_module::dsl as m;

        let query = m::tenant_module
            .filter(m::tenant_id.eq(tenant_id))
            .filter(m::module.eq(module))
            .select(TenantModuleRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding tenant module")
            .into_db_result()
    }

    /// Enabling an enabled module keeps its first `enabled_at`.
    pub async fn enable(
        conn: &mut PgConn,
        tenant_id: TenantId,
        module: &str,
    ) -> DbResult<TenantModuleRow> {
        use crate::schema::tenant_module::dsl as m;

        let query = diesel::insert_into(m::tenant_module)
            .values((m::tenant_id.eq(tenant_id), m::module.eq(module)))
            .on_conflict((m::tenant_id, m::module))
            .do_update()
            .set(m::enabled_at.eq(m::enabled_at))
            .returning(TenantModuleRow::as_returning());
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while enabling tenant module")
            .into_db_result()
    }

    /// Whether the module was enabled.
    pub async fn disable(conn: &mut PgConn, tenant_id: TenantId, module: &str) -> DbResult<bool> {
        use crate::schema::tenant_module::dsl as m;

        let query = diesel::delete(m::tenant_module)
            .filter(m::tenant_id.eq(tenant_id))
            .filter(m::module.eq(module));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|deleted| deleted > 0)
            .attach("Error while disabling tenant module")
            .into_db_result()
    }
}
