use crate::api_tokens::{ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::TenantId;
use diesel::{ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl ApiTokenRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ApiTokenRow> {
        use crate::schema::api_token::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(api_token).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting api token")
            .into_db_result()
    }
}

impl ApiTokenRow {
    pub async fn find_by_id(conn: &mut PgConn, param_id: &uuid::Uuid) -> DbResult<ApiTokenRow> {
        use crate::schema::api_token::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = api_token.filter(id.eq(param_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while fetching api token by id")
            .into_db_result()
    }

    pub async fn find_by_tenant_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
    ) -> DbResult<Vec<ApiTokenRow>> {
        use crate::schema::api_token::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = api_token.filter(tenant_id.eq(param_tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching api tokens by tenant id")
            .into_db_result()
    }
}

impl ApiTokenValidationRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        api_token_id: &uuid::Uuid,
    ) -> DbResult<ApiTokenValidationRow> {
        use crate::schema::api_token::dsl as at_dsl;
        use crate::schema::tenant::dsl as t_dsl;
        use diesel_async::RunQueryDsl;

        let query = at_dsl::api_token
            .inner_join(t_dsl::tenant.on(t_dsl::id.eq(at_dsl::tenant_id)))
            .filter(at_dsl::id.eq(api_token_id))
            .select(ApiTokenValidationRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while fetching api token by id")
            .into_db_result()
    }
}
