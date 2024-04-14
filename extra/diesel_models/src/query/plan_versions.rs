use crate::errors::IntoDbResult;
use crate::plan_versions::{PlanVersion, PlanVersionNew};

use crate::{DbResult, PgConn};

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
            .attach_printable("Error while inserting plan version")
            .into_db_result()
    }
}
