use crate::errors::IntoDbResult;
use crate::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
    PaymentTransactionWithMethodRow,
};
use crate::{DbResult, PgConn};

use crate::enums::PaymentStatusEnum;
use common_domain::ids::{
    CheckoutSessionId, InvoiceId, PaymentTransactionId, StoredDocumentId, TenantId,
};
use diesel::prelude::{
    ExpressionMethods, NullableExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper,
};
use diesel::{JoinOnDsl, debug_query};
use error_stack::ResultExt;

/// Payment states that own their invoice: money is either moving or has landed.
/// Used by the consolidation fence; [`PaymentTransactionRow::exists_live_for_invoice`]
/// applies the same statuses but nets refunds out of Settled amounts.
pub const LIVE_PAYMENT_STATUSES: [PaymentStatusEnum; 3] = [
    PaymentStatusEnum::Pending,
    PaymentStatusEnum::Ready,
    PaymentStatusEnum::Settled,
];

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

    /// True when the invoice has a payment attempt that owns it: in flight
    /// (Pending/Ready), or settled money whose net amount still covers the
    /// invoice's collectible total.
    ///
    /// An invoice matching this must never be re-charged, merged into a
    /// consolidated parent, or have its lines mutated — all three would act on
    /// an amount the provider is already moving money against. Settled
    /// amounts are netted of refunds, mirroring
    /// [`InvoiceRow::recompute_amount_due_from_settled_payments`]: a partial
    /// refund reopens a balance that must stay collectible, so a
    /// Settled-but-partially-refunded payment stops owning the invoice once
    /// its net amount no longer covers the total.
    ///
    /// [`InvoiceRow::recompute_amount_due_from_settled_payments`]: crate::invoices::InvoiceRow::recompute_amount_due_from_settled_payments
    pub async fn exists_live_for_invoice(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<bool> {
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel::dsl::{exists, select};
        use diesel_async::RunQueryDsl;

        let in_flight_query = select(exists(
            pt_dsl::payment_transaction
                .filter(pt_dsl::invoice_id.eq(inv_uid))
                .filter(pt_dsl::tenant_id.eq(tenant_uid))
                .filter(
                    pt_dsl::status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready]),
                )
                // Same as the settled branch: only a Payment attempt owns the
                // invoice. A Pending/Ready refund/adjustment moves money the
                // other way and must not fence the collectible balance.
                .filter(pt_dsl::payment_type.eq(crate::enums::PaymentTypeEnum::Payment)),
        ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&in_flight_query));

        let in_flight: bool = in_flight_query
            .get_result(conn)
            .await
            .attach("Error while checking for in-flight payment transactions")
            .into_db_result()?;
        if in_flight {
            return Ok(true);
        }

        let settled_amounts: Vec<(i64, i64)> = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq(PaymentStatusEnum::Settled))
            .filter(pt_dsl::payment_type.eq(crate::enums::PaymentTypeEnum::Payment))
            .select((pt_dsl::amount, pt_dsl::amount_refunded))
            .load(conn)
            .await
            .attach("Error while summing settled payment transactions")
            .into_db_result()?;
        let settled_net: i64 = settled_amounts
            .iter()
            .map(|(amount, refunded)| amount - refunded)
            .sum();
        if settled_net <= 0 {
            return Ok(false);
        }

        let (total, applied_credits): (i64, i64) = i_dsl::invoice
            .filter(i_dsl::id.eq(inv_uid))
            .filter(i_dsl::tenant_id.eq(tenant_uid))
            .select((i_dsl::total, i_dsl::applied_credits))
            .first(conn)
            .await
            .attach("Error while loading invoice total for live-payment check")
            .into_db_result()?;

        Ok(settled_net >= total - applied_credits)
    }

    /// Number of attempts against this invoice that ended without collecting anything.
    /// Drives the dunning ladder, so it must count attempts, not events: a redelivered
    /// webhook re-derives the same number instead of advancing the schedule.
    pub async fn count_failed_for_invoice(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(
                pt_dsl::status.eq_any([PaymentStatusEnum::Failed, PaymentStatusEnum::Cancelled]),
            )
            // A reversal is a settled payment clawed back later, not a failed attempt.
            .filter(pt_dsl::refunded_at.is_null())
            .count();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while counting failed payment transactions")
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

    /// Resolve a transaction by the provider's own charge id, scoped to the
    /// tenant. Used by the webhook settlement path when a GoCardless event
    /// carries no `meteroid.transaction_id` (empty resource metadata): the GC
    /// payment id (`links.payment`) was persisted as `provider_transaction_id`
    /// at charge time, so it recovers the local transaction.
    pub async fn get_by_provider_transaction_id(
        conn: &mut PgConn,
        provider_tx_id: &str,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            payment_transaction, provider_transaction_id, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(provider_transaction_id.eq(provider_tx_id))
            .filter(tenant_id.eq(tenant_uid))
            .select(PaymentTransactionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding transaction by provider id")
            .into_db_result()
    }

    /// Pending transactions older than `older_than` (by `created_at`) that
    /// have a known `provider_transaction_id`. The reconciliation worker
    /// iterates this list, polling each provider to find out what really
    /// happened — covers the case where the webhook for a charge never
    /// arrived.
    ///
    /// The age filter excludes fresh-Pending rows whose webhook is simply
    /// still in flight; without it the worker would burn provider API rate
    /// limit on every newly-created transaction. The partial index
    /// `idx_payment_transaction_pending_created_at` keeps the query fast
    /// even with millions of historical rows. Returns oldest-first so a
    /// stuck transaction doesn't get starved by a flood of newer ones.
    pub async fn list_pending_with_provider_id(
        conn: &mut PgConn,
        older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            created_at, payment_transaction, provider_transaction_id, status,
        };
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(status.eq(PaymentStatusEnum::Pending))
            .filter(provider_transaction_id.is_not_null())
            .filter(created_at.lt(older_than))
            .order(created_at.asc())
            .limit(limit)
            .select(PaymentTransactionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error listing pending transactions")
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

impl PaymentTransactionRow {
    /// The most recent non-terminal (Pending/Ready) transaction for a checkout
    /// session, if any. Makes checkout completion idempotent: a re-invocation
    /// while a charge is in flight returns the existing transaction instead of
    /// issuing a fresh charge (which would mint a new idempotency key and
    /// double-charge at the provider).
    pub async fn get_active_by_checkout_session_id(
        conn: &mut PgConn,
        checkout_session_uid: CheckoutSessionId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            checkout_session_id, created_at, payment_transaction, status, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(checkout_session_id.eq(checkout_session_uid))
            .filter(tenant_id.eq(tenant_uid))
            .filter(status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready]))
            .order(created_at.desc())
            .select(PaymentTransactionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding active checkout transaction")
            .into_db_result()
    }

    /// The most recent transaction for a checkout session, at ANY status. Unlike
    /// [`Self::get_active_by_checkout_session_id`] (Pending/Ready only), this also
    /// sees a tx that already Settled — needed by hosted-checkout fulfillment,
    /// which must still materialize the subscription when `payments.confirmed`
    /// beat `billing_requests.fulfilled` and drove the tx to Settled first.
    pub async fn get_latest_by_checkout_session_id(
        conn: &mut PgConn,
        checkout_session_uid: CheckoutSessionId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            checkout_session_id, created_at, payment_transaction, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(checkout_session_id.eq(checkout_session_uid))
            .filter(tenant_id.eq(tenant_uid))
            .order(created_at.desc())
            .select(PaymentTransactionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding latest checkout transaction")
            .into_db_result()
    }

    /// Tag a transaction with the checkout session it belongs to. The
    /// checkout-activation charge is linked to an invoice, so its row does not
    /// otherwise carry the session id; tagging it lets the completion
    /// idempotency guard find it by `checkout_session_id` on a re-completion.
    pub async fn set_checkout_session_id(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
        checkout_session_uid: CheckoutSessionId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::{
            checkout_session_id, id, payment_transaction, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::update(
            payment_transaction
                .filter(id.eq(tx_id))
                .filter(tenant_id.eq(tenant_uid)),
        )
        .set(checkout_session_id.eq(checkout_session_uid));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach("Error while tagging transaction with checkout session")
            .into_db_result()
    }
}
