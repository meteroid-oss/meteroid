use crate::errors::IntoDbResult;
use crate::tax_categories::{TaxCategoryRow, TaxCategoryRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{TaxCategoryId, TenantId};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper,
    debug_query,
};
use error_stack::ResultExt;

impl TaxCategoryRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<TaxCategoryRow> {
        use crate::schema::tax_category::dsl as tc;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(tc::tax_category)
            .values(self)
            .returning(TaxCategoryRow::as_returning());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting tax category")
            .into_db_result()
    }
}

impl TaxCategoryRow {
    /// Built-in (global, tenant_id NULL) categories plus this tenant's own.
    pub async fn list_available(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<TaxCategoryRow>> {
        use crate::schema::tax_category::dsl as tc;
        use diesel_async::RunQueryDsl;

        let query = tc::tax_category
            .filter(tc::tenant_id.is_null().or(tc::tenant_id.eq(tenant_id)))
            .select(TaxCategoryRow::as_select())
            .order(tc::name.asc());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing tax categories")
            .into_db_result()
    }

    /// Whether the category exists and is usable by this tenant (built-in or its own).
    pub async fn is_available_for_tenant(
        conn: &mut PgConn,
        id: TaxCategoryId,
        tenant_id: TenantId,
    ) -> DbResult<bool> {
        use crate::schema::tax_category::dsl as tc;
        use diesel_async::RunQueryDsl;

        let query = diesel::select(diesel::dsl::exists(
            tc::tax_category
                .filter(tc::id.eq(id))
                .filter(tc::tenant_id.is_null().or(tc::tenant_id.eq(tenant_id))),
        ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while checking tax category availability")
            .into_db_result()
    }

    /// Renames/reparents a tenant-owned custom category. Built-in (global) rows
    /// and other tenants' rows are never matched, so nothing returns.
    pub async fn update_details(
        conn: &mut PgConn,
        id: TaxCategoryId,
        tenant_id: TenantId,
        name: String,
        parent_id: Option<TaxCategoryId>,
    ) -> DbResult<Option<TaxCategoryRow>> {
        use crate::schema::tax_category::dsl as tc;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(tc::tax_category)
            .filter(tc::id.eq(id))
            .filter(tc::tenant_id.eq(tenant_id))
            .filter(tc::is_builtin.eq(false))
            .set((tc::name.eq(name), tc::parent_id.eq(parent_id)))
            .returning(TaxCategoryRow::as_returning());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating tax category")
            .into_db_result()
    }

    /// Deletes a tenant-owned custom category. Built-in (global) rows and other
    /// tenants' rows are never matched. Returns the number of rows removed.
    pub async fn delete(
        conn: &mut PgConn,
        id: TaxCategoryId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::tax_category::dsl as tc;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(tc::tax_category)
            .filter(tc::id.eq(id))
            .filter(tc::tenant_id.eq(tenant_id))
            .filter(tc::is_builtin.eq(false));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while deleting tax category")
            .into_db_result()
    }
}
