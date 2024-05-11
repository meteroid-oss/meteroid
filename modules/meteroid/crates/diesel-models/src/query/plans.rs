use crate::errors::IntoDbResult;
use crate::plans::{Plan, PlanNew};

use crate::{DbResult, PgConn};

use diesel::{debug_query, ExpressionMethods, QueryDsl};
use error_stack::ResultExt;
use uuid::Uuid;

impl PlanNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Plan> {
        use crate::schema::plan::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting plan")
            .into_db_result()
    }
}

impl Plan {
    pub async fn get_by_external_id_and_tenant_id(
        conn: &mut PgConn,
        external_id: &str,
        tenant_id: Uuid,
    ) -> DbResult<Plan> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::external_id.eq(external_id))
            .filter(p_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan")
            .into_db_result()
    }
}
