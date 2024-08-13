use crate::customer_balance_txs::{
    CustomerBalancePendingTxRow, CustomerBalancePendingTxRowNew, CustomerBalanceTxRow,
    CustomerBalanceTxRowNew,
};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use diesel::debug_query;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl CustomerBalanceTxRowNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<CustomerBalanceTxRow> {
        use crate::schema::customer_balance_tx::dsl as cbtx_dsl;

        let query = diesel::insert_into(cbtx_dsl::customer_balance_tx).values(&self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting customer balance tx")
            .into_db_result()
    }
}

impl CustomerBalanceTxRow {}

impl CustomerBalancePendingTxRowNew {
    pub async fn insert(self, conn: &mut PgConn) -> DbResult<CustomerBalancePendingTxRow> {
        use crate::schema::customer_balance_pending_tx::dsl as cbtx_dsl;

        let query = diesel::insert_into(cbtx_dsl::customer_balance_pending_tx).values(&self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting customer balance pending tx")
            .into_db_result()
    }
}
