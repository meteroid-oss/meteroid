use crate::bi::{BiMrrMovementLog, BiMrrMovementLogNew};
use crate::errors::IntoDbResult;

use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

impl BiMrrMovementLogNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<BiMrrMovementLog> {
        use crate::schema::bi_mrr_movement_log::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(bi_mrr_movement_log).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting bi_mrr_movement_log")
            .into_db_result()
    }
}
impl BiMrrMovementLog {
    pub async fn insert_movement_log_batch(
        conn: &mut PgConn,
        invoices: Vec<BiMrrMovementLogNew>,
    ) -> DbResult<Vec<BiMrrMovementLog>> {
        use crate::schema::bi_mrr_movement_log::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(bi_mrr_movement_log).values(&invoices);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting bi_mrr_movement_log")
            .into_db_result()
    }
}
