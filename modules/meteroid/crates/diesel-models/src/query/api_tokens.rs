use diesel::ExpressionMethods;
use diesel::{debug_query, QueryDsl};
use error_stack::ResultExt;

use crate::api_tokens::{ApiToken, ApiTokenNew};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

impl ApiTokenNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<ApiToken> {
        use crate::schema::api_token::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(api_token).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting api token")
            .into_db_result()
    }
}

impl ApiToken {
    pub async fn find_by_id(conn: &mut PgConn, param_id: &uuid::Uuid) -> DbResult<ApiToken> {
        use crate::schema::api_token::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = api_token.filter(id.eq(param_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while fetching api token by id")
            .into_db_result()
    }

    pub async fn find_by_tenant_id(
        conn: &mut PgConn,
        param_tenant_id: &uuid::Uuid,
    ) -> DbResult<Vec<ApiToken>> {
        use crate::schema::api_token::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = api_token.filter(tenant_id.eq(param_tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching api tokens by tenant id")
            .into_db_result()
    }
}
