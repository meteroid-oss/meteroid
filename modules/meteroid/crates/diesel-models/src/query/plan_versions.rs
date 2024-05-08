use crate::errors::IntoDbResult;
use crate::plan_versions::{PlanVersion, PlanVersionNew};

use crate::{DbResult, PgConn};

use diesel::debug_query;
use diesel::prelude::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl PlanVersionNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan_version).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting plan version")
            .into_db_result()
    }
}

impl PlanVersion {
    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding plan version by id")
            .into_db_result()
    }
}
