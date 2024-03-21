use crate::billable_metrics::{BillableMetric, BillableMetricNew};
use crate::errors::IntoDbResult;
use crate::schema::billable_metric;
use crate::{errors, DbResult, PgConn};
use diesel::associations::HasTable;
use diesel::debug_query;
use error_stack::ResultExt;

impl BillableMetricNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<BillableMetric> {
        use crate::schema::billable_metric::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(billable_metric).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting billable metric")
            .into_db_result()
    }
}
