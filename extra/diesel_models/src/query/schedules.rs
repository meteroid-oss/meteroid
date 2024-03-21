use crate::errors::IntoDbResult;
use crate::schedules::{Schedule, ScheduleNew};
use crate::schema::schedule;
use crate::{errors, DbResult, PgConn};
use diesel::associations::HasTable;
use diesel::debug_query;
use error_stack::ResultExt;

impl ScheduleNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Schedule> {
        use crate::schema::schedule::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(schedule).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting schedule")
            .into_db_result()
    }
}
