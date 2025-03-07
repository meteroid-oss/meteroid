use crate::errors::IntoDbResult;
use crate::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
};
use crate::{DbResult, PgConn};

use common_domain::ids::{InvoiceId, PaymentTransactionId, TenantId};
use diesel::debug_query;
use diesel::prelude::{ExpressionMethods, QueryDsl};
use error_stack::ResultExt;

impl PaymentTransactionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(payment_transaction).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting connector")
            .into_db_result()
    }
}

impl PaymentTransactionRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(id.eq(tx_id))
            .filter(tenant_id.eq(tenant_uid));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding transaction")
            .into_db_result()
    }

    pub async fn list_by_invoice_id(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(invoice_id.eq(inv_uid))
            .filter(tenant_id.eq(tenant_uid));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding transaction")
            .into_db_result()
    }
}

impl PaymentTransactionRowPatch {
    pub async fn update(&self, conn: &mut PgConn) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(payment_transaction.filter(id.eq(self.id))).set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating transaction")
            .into_db_result()
    }
}
