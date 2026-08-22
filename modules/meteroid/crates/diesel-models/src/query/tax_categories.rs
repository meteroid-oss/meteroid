use crate::errors::IntoDbResult;
use crate::tax_categories::TaxCategoryRow;
use crate::{DbResult, PgConn};
use common_domain::ids::{TaxCategoryId, TenantId};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

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
}
