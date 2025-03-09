use crate::customer_balance_txs::{
    CustomerBalancePendingTxRow, CustomerBalancePendingTxRowNew, CustomerBalanceTxRow,
    CustomerBalanceTxRowNew,
};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::InvoiceId;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, debug_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

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

impl CustomerBalancePendingTxRow {
    pub async fn find_unprocessed_by_invoice_id(
        conn: &mut PgConn,
        invoice_id: InvoiceId,
    ) -> DbResult<Option<CustomerBalancePendingTxRow>> {
        use crate::schema::customer_balance_pending_tx::dsl as cbptx;

        let query = cbptx::customer_balance_pending_tx
            .filter(cbptx::invoice_id.eq(invoice_id))
            .filter(cbptx::tx_id.is_null());
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .optional()
            .attach_printable("Error while finding CustomerBalancePendingTx by invoice_id")
            .into_db_result()
    }

    pub async fn update_tx_id(conn: &mut PgConn, id: Uuid, tx_id: Uuid) -> DbResult<usize> {
        use crate::schema::customer_balance_pending_tx::dsl as cbptx;

        let query = diesel::update(cbptx::customer_balance_pending_tx)
            .filter(cbptx::id.eq(id))
            .set((
                cbptx::tx_id.eq(tx_id),
                cbptx::updated_at.eq(diesel::dsl::now),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while update_tx_id")
            .into_db_result()
    }
}
