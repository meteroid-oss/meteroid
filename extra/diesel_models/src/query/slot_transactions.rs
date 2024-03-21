use crate::errors::IntoDbResult;
use crate::schema::slot_transaction;
use crate::slot_transactions::SlotTransaction;
use crate::{errors, DbResult, PgConn};
use diesel::associations::HasTable;
use diesel::debug_query;
use error_stack::ResultExt;

impl SlotTransaction {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SlotTransaction> {
        use crate::schema::slot_transaction::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(slot_transaction).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting slot transaction")
            .into_db_result()
    }
}
