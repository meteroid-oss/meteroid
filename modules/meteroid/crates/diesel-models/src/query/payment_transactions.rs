use crate::errors::IntoDbResult;
use crate::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
    PaymentTransactionWithMethodRow,
};
use crate::{DbResult, PgConn};

use crate::enums::PaymentStatusEnum;
use common_domain::ids::{InvoiceId, PaymentTransactionId, StoredDocumentId, TenantId};
use diesel::prelude::{
    ExpressionMethods, NullableExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper,
};
use diesel::{JoinOnDsl, debug_query};
use error_stack::ResultExt;

impl PaymentTransactionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::payment_transaction;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(payment_transaction).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while inserting connector")
            .into_db_result()
    }
}

impl PaymentTransactionRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::{id, payment_transaction, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(id.eq(tx_id))
            .filter(tenant_id.eq(tenant_uid));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding transaction")
            .into_db_result()
    }

    pub async fn get_by_id_for_update(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::{id, payment_transaction, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(id.eq(tx_id))
            .filter(tenant_id.eq(tenant_uid))
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach("Error while finding transaction")
            .into_db_result()
    }

    pub async fn list_by_invoice_id(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<Vec<PaymentTransactionWithMethodRow>> {
        use crate::schema::customer_payment_method::dsl as cpm_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .left_join(
                cpm_dsl::customer_payment_method
                    .on(pt_dsl::payment_method_id.eq(cpm_dsl::id.nullable())),
            );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while finding transaction")
            .into_db_result()
    }

    pub async fn last_settled_by_invoice_id(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            invoice_id, payment_transaction, processed_at, status, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(invoice_id.eq(inv_uid))
            .filter(tenant_id.eq(tenant_uid))
            .filter(status.eq(PaymentStatusEnum::Settled))
            .order(processed_at.desc())
            .select(PaymentTransactionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding transaction")
            .into_db_result()
    }

    pub async fn set_receipt_pdf(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
        pdf_id: StoredDocumentId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::{
            id, payment_transaction, receipt_pdf_id, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::update(payment_transaction.filter(id.eq(tx_id)))
            .filter(tenant_id.eq(tenant_uid))
            .set(receipt_pdf_id.eq(pdf_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while setting receipt PDF")
            .into_db_result()
    }
}

impl PaymentTransactionRowPatch {
    pub async fn update(&self, conn: &mut PgConn) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::{id, payment_transaction};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(payment_transaction.filter(id.eq(self.id))).set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while updating transaction")
            .into_db_result()
    }

    pub async fn patch(
        &self,
        conn: &mut PgConn,
        tenant_uid: TenantId,
        tx_id: PaymentTransactionId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::{id, payment_transaction, tenant_id};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(
            payment_transaction
                .filter(id.eq(tx_id))
                .filter(tenant_id.eq(tenant_uid)),
        )
        .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while patching transaction")
            .into_db_result()
    }
}
