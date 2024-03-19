use crate::errors::IntoDbResult;
use crate::plan_versions::{PlanVersion, PlanVersionNew};
use crate::schema::plan_version;
use crate::{errors, DbResult, PgConn};
use diesel::associations::HasTable;
use diesel::debug_query;
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
            .into_db_result()
            .attach_printable("Error while inserting plan")
    }
}
