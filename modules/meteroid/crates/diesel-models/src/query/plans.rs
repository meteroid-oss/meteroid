use crate::errors::IntoDbResult;
use crate::plans::{Plan, PlanNew};

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

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
