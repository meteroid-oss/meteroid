use crate::bi::{BiMrrMovementLogRow, BiMrrMovementLogRowNew};
use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl BiMrrMovementLogRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<BiMrrMovementLogRow> {
        use crate::schema::bi_mrr_movement_log::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(bi_mrr_movement_log).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting bi_mrr_movement_log")
            .into_db_result()
    }
}
impl BiMrrMovementLogRow {
    pub async fn insert_movement_log_batch(
        conn: &mut PgConn,
        invoices: Vec<BiMrrMovementLogRowNew>,
    ) -> DbResult<Vec<BiMrrMovementLogRow>> {
        use crate::schema::bi_mrr_movement_log::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(bi_mrr_movement_log).values(&invoices);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting bi_mrr_movement_log")
            .into_db_result()
    }
}
