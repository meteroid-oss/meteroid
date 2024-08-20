use crate::errors::IntoDbResult;
use crate::tenants::{TenantRow, TenantRowNew};
use crate::{DbResult, PgConn};

use diesel::prelude::{ExpressionMethods, QueryDsl};
use diesel::{debug_query, JoinOnDsl, SelectableHelper};
use error_stack::ResultExt;

impl TenantRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(tenant).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting tenant")
            .into_db_result()
    }
}

impl TenantRow {
    pub async fn find_by_id(conn: &mut PgConn, tenant_id: uuid::Uuid) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = tenant.filter(id.eq(tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding tenant by id")
            .into_db_result()
    }

    pub async fn find_by_slug_and_organization_slug(conn: &mut PgConn, param_tenant_slug: String, organization_slug: String) -> DbResult<TenantRow> {
        use crate::schema::tenant::dsl as t_dsl;
        use crate::schema::organization::dsl as o_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .inner_join(o_dsl::organization.on(t_dsl::organization_id.eq(o_dsl::id)))
            .filter(t_dsl::slug.eq(param_tenant_slug))
            .filter(o_dsl::slug.eq(organization_slug))
            .select(TenantRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding tenant by slug")
            .into_db_result()
    }

    pub async fn list_by_organization_id(
        conn: &mut PgConn,
        organization_id: uuid::Uuid,
    ) -> DbResult<Vec<TenantRow>> {
        use crate::schema::organization::dsl as o_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = t_dsl::tenant
            .inner_join(o_dsl::organization.on(t_dsl::organization_id.eq(o_dsl::id)))
            .filter(o_dsl::id.eq(organization_id))
            .select(TenantRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching tenants by user_id")
            .into_db_result()
    }
}
