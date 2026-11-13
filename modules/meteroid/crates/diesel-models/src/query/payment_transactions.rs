use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
    PaymentTransactionWithMethodRow,
};
use crate::{DbResult, PgConn};

use crate::enums::{InvoicePaymentStatus, InvoiceStatusEnum, PaymentStatusEnum, PaymentTypeEnum};
use common_domain::ids::{
    CheckoutSessionId, CreditNoteId, CustomerId, InvoiceId, PaymentTransactionId, StoredDocumentId,
    SubscriptionId, TenantId,
};
use diesel::prelude::{
    ExpressionMethods, NullableExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper,
};
use diesel::{JoinOnDsl, PgSortExpressionMethods};
use error_stack::ResultExt;

/// Payment states that own their invoice: money is either moving or has landed.
/// Used by the consolidation fence; [`PaymentTransactionRow::exists_live_for_invoice`]
/// applies the same statuses but nets involuntary claw-backs out of Settled amounts.
pub const LIVE_PAYMENT_STATUSES: [PaymentStatusEnum; 3] = [
    PaymentStatusEnum::Pending,
    PaymentStatusEnum::Ready,
    PaymentStatusEnum::Settled,
];

impl PaymentTransactionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::payment_transaction;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(payment_transaction).values((
            self,
            crate::schema::payment_transaction::created_at.eq(common_domain::clock::now()),
        ));

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding transaction")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{id, payment_transaction, tenant_id};
        use diesel::OptionalExtension;
        use diesel_async::RunQueryDsl;

        payment_transaction
            .filter(id.eq(tx_id))
            .filter(tenant_id.eq(tenant_uid))
            .select(PaymentTransactionRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding payment transaction")
            .into_db_result()
    }

    /// The provider processes a pending payment without the customer now: no action is awaited.
    pub async fn clear_awaited_action(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(pt_dsl::payment_transaction)
            .filter(pt_dsl::id.eq(tx_id))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready]))
            .filter(pt_dsl::next_action.is_not_null())
            .set(pt_dsl::next_action.eq(None::<serde_json::Value>))
            .execute(conn)
            .await
            .attach("Error while clearing the awaited action of a payment")
            .into_db_result()
    }

    /// The customer a payment collects from: its invoice's, checkout session's, credit note's,
    /// parent payment's, payment method's or top-up's.
    pub async fn customer_of(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        tx_id: PaymentTransactionId,
    ) -> DbResult<CustomerId> {
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Owner {
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
            customer_id: Option<CustomerId>,
        }
        let owner: Owner = diesel::sql_query(
            "SELECT COALESCE(i.customer_id, cs.customer_id, cn.customer_id, pi.customer_id, \
                    cpm.customer_id, p.initiated_by_customer_id, \
                    (SELECT o.customer_id FROM customer_credit_operation o \
                      WHERE o.tenant_id = p.tenant_id AND o.payment_transaction_id = p.id LIMIT 1), \
                    (SELECT o.customer_id FROM customer_credit_operation o \
                      WHERE o.tenant_id = p.tenant_id \
                        AND o.metadata @> jsonb_build_object('superseded_payment_transaction_ids', \
                                                             jsonb_build_array(p.id::text)) \
                      LIMIT 1)) AS customer_id \
             FROM payment_transaction p \
             LEFT JOIN invoice i ON i.id = p.invoice_id \
             LEFT JOIN checkout_session cs ON cs.id = p.checkout_session_id \
             LEFT JOIN credit_note cn ON cn.id = p.credit_note_id \
             LEFT JOIN payment_transaction parent ON parent.id = p.parent_transaction_id \
             LEFT JOIN invoice pi ON pi.id = parent.invoice_id \
             LEFT JOIN customer_payment_method cpm ON cpm.id = p.payment_method_id \
             WHERE p.id = $1 AND p.tenant_id = $2",
        )
        .bind::<diesel::sql_types::Uuid, _>(tx_id)
        .bind::<diesel::sql_types::Uuid, _>(tenant_uid)
        .get_result(conn)
        .await
        .attach("Error while reading the customer of a payment")
        .into_db_result()?;
        owner.customer_id.ok_or_else(|| {
            crate::errors::DatabaseError::Others(format!("payment {tx_id} has no customer")).into()
        })
    }

    /// Locks the payment's customer, then the payment (`crate::lock`).
    pub async fn lock_with_customer(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        tx_id: PaymentTransactionId,
    ) -> DbResult<(CustomerLock, PaymentTransactionRow)> {
        let customer = Self::customer_of(conn, tenant_uid, tx_id).await?;
        let lock = crate::customers::CustomerRow::lock(conn, tenant_uid, customer).await?;
        let row = Self::get_by_id_for_update(conn, &lock, tx_id).await?;
        Ok((lock, row))
    }

    pub async fn get_by_id_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        tx_id: PaymentTransactionId,
    ) -> DbResult<PaymentTransactionRow> {
        use crate::schema::payment_transaction::dsl::{id, payment_transaction, tenant_id};
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        let query = payment_transaction
            .filter(id.eq(tx_id))
            .filter(tenant_id.eq(lock.tenant_id()))
            .for_update();

        crate::debug_sql(&query);

        let row: PaymentTransactionRow = query
            .first(conn)
            .await
            .attach("Error while finding transaction")
            .into_db_result()?;
        let owner = Self::customer_of(conn, lock.tenant_id(), tx_id).await?;
        lock.covers(conn, row.tenant_id, owner)?;
        Ok(row)
    }

    /// Any payment attempt, whatever its status or outcome, was ever made for the invoice.
    pub async fn exists_for_invoice(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<bool> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel::dsl::{exists, select};
        use diesel_async::RunQueryDsl;

        let query = select(exists(
            pt_dsl::payment_transaction
                .filter(pt_dsl::invoice_id.eq(inv_uid))
                .filter(pt_dsl::tenant_id.eq(tenant_uid)),
        ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while checking for payment transactions")
            .into_db_result()
    }

    /// True when a payment attempt owns the invoice: in flight (Pending/Ready), or settled money
    /// that covers the collectible total, netted like `recompute_amount_due_from_settled_payments`
    /// (involuntary claw-backs, finalized invoice-reducing credit notes). Never re-charge, merge
    /// or mutate such an invoice.
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
                // Same as the settled branch: only a collection attempt owns the
                // invoice. A Pending/Ready refund/adjustment moves money the
                // other way and must not fence the collectible balance.
                .filter(pt_dsl::payment_type.eq_any(PaymentTypeEnum::COLLECTIONS)),
        ));

        crate::debug_sql(&in_flight_query);

        let in_flight: bool = in_flight_query
            .get_result(conn)
            .await
            .attach("Error while checking for in-flight payment transactions")
            .into_db_result()?;
        if in_flight {
            return Ok(true);
        }

        let settled_amounts: Vec<(i64, i64, i64)> = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(
                pt_dsl::status.eq_any([PaymentStatusEnum::Settled, PaymentStatusEnum::Refunded]),
            )
            .filter(pt_dsl::payment_type.eq_any(PaymentTypeEnum::COLLECTIONS))
            .select((
                pt_dsl::amount,
                pt_dsl::amount_reversed,
                pt_dsl::amount_disputed,
            ))
            .load(conn)
            .await
            .attach("Error while summing settled payment transactions")
            .into_db_result()?;
        let refunded_against_notes =
            Self::sum_credit_note_backed_refunds_by_invoice(conn, inv_uid, tenant_uid).await?;
        // Money lost to a dispute still paid the invoice.
        let settled_net: i64 = settled_amounts
            .iter()
            .map(|(amount, reversed, disputed)| amount - (reversed - disputed))
            .sum::<i64>()
            - refunded_against_notes;
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

        // Finalized invoice-reducing credit notes shrink the collectible total (same
        // netting as recompute), so a settled payment covering only the remainder
        // still owns the invoice. Requires settled_net > 0 (gated above).
        let cancelled_sum = crate::credit_notes::CreditNoteRow::sum_finalized_invoice_reducing(
            conn, inv_uid, tenant_uid,
        )
        .await?;

        Ok(settled_net >= (total - applied_credits) - cancelled_sum)
    }

    /// Number of genuinely Failed (declined) collection attempts against this invoice.
    /// Drives the dunning ladder, so it counts attempts, not events (a redelivered webhook
    /// re-derives the same number), and excludes Cancelled: aborted, not declined.
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
            .filter(pt_dsl::status.eq(PaymentStatusEnum::Failed))
            // A reversal is a settled payment clawed back later, not a failed attempt.
            .filter(pt_dsl::refunded_at.is_null())
            .count();

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting failed payment transactions")
            .into_db_result()
    }

    pub async fn latest_failed_for_invoice(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Payment))
            .filter(pt_dsl::status.eq(PaymentStatusEnum::Failed))
            .filter(pt_dsl::refunded_at.is_null())
            .order_by(pt_dsl::created_at.desc())
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching the latest failed payment transaction")
            .into_db_result()
    }

    /// The latest failed attempt of each of the customer's errored subscription invoices.
    pub async fn latest_failed_for_errored_invoices_of_customer(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        customer_uid: CustomerId,
    ) -> DbResult<Vec<(SubscriptionId, PaymentTransactionRow)>> {
        use crate::schema::invoice::dsl as i_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .inner_join(i_dsl::invoice.on(pt_dsl::invoice_id.eq(i_dsl::id.nullable())))
            .filter(i_dsl::tenant_id.eq(tenant_uid))
            .filter(i_dsl::customer_id.eq(customer_uid))
            .filter(i_dsl::status.eq(InvoiceStatusEnum::Finalized))
            .filter(i_dsl::payment_status.eq(InvoicePaymentStatus::Errored))
            .filter(i_dsl::amount_due.gt(0))
            .filter(i_dsl::subscription_id.is_not_null())
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Payment))
            .filter(pt_dsl::status.eq(PaymentStatusEnum::Failed))
            .filter(pt_dsl::refunded_at.is_null())
            .distinct_on(i_dsl::id)
            .order_by((i_dsl::id, pt_dsl::created_at.desc()))
            .select((
                i_dsl::subscription_id.assume_not_null(),
                PaymentTransactionRow::as_select(),
            ));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing failed payments of errored invoices")
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

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while finding transaction")
            .into_db_result()
    }

    /// Of `invoice_ids`, those a collection ever settled on: still Settled, or Refunded since.
    pub async fn invoices_with_collection(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        invoice_ids: &[InvoiceId],
    ) -> DbResult<Vec<InvoiceId>> {
        use crate::schema::payment_transaction::dsl::{
            invoice_id, payment_transaction, payment_type, status, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        if invoice_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = payment_transaction
            .filter(tenant_id.eq(tenant_uid))
            .filter(invoice_id.eq_any(invoice_ids))
            .filter(status.eq_any([PaymentStatusEnum::Settled, PaymentStatusEnum::Refunded]))
            .filter(payment_type.eq_any(PaymentTypeEnum::COLLECTIONS))
            .select(invoice_id.assume_not_null())
            .distinct();

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while finding collected invoices")
            .into_db_result()
    }

    /// The invoice's most recent settled PAYMENT — its receipt. REFUND rows share the
    /// invoice and settle later, so they would otherwise shadow the payment they give back.
    pub async fn last_settled_by_invoice_id(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            invoice_id, payment_transaction, payment_type, processed_at, status, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(invoice_id.eq(inv_uid))
            .filter(tenant_id.eq(tenant_uid))
            .filter(status.eq(PaymentStatusEnum::Settled))
            .filter(payment_type.eq_any(PaymentTypeEnum::COLLECTIONS))
            .order(processed_at.desc().nulls_last())
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding transaction by provider id")
            .into_db_result()
    }

    /// Claim up to `limit` Pending rows of `kind` with a provider id for the reconcile sweep, and
    /// stamp `last_reconciled_at` so they are not polled again until due. Rows younger than
    /// `older_than` are skipped: their webhook may simply still be in flight. A row is due again
    /// after a tenth of its age, between one minute and one hour, so a charge waiting a day on
    /// a batch capture costs tens of polls, not one a minute. Least recently polled first, so
    /// rows that never resolve cannot starve the rest.
    pub async fn claim_pending_for_reconcile(
        conn: &mut PgConn,
        kind: PaymentTypeEnum,
        older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            created_at, id, last_reconciled_at, payment_transaction, payment_type,
            provider_transaction_id, status,
        };
        use diesel::dsl::sql;
        use diesel::sql_types::{Bool, Timestamptz};
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(status.eq(PaymentStatusEnum::Pending))
            .filter(payment_type.eq(kind))
            .filter(provider_transaction_id.is_not_null())
            .filter(created_at.lt(older_than))
            .filter(sql::<Bool>(
                "(last_reconciled_at IS NULL OR last_reconciled_at < NOW() - \
                 LEAST(GREATEST((NOW() - created_at) / 10, INTERVAL '1 minute'), INTERVAL '1 hour'))",
            ))
            .order(sql::<Timestamptz>("COALESCE(last_reconciled_at, created_at)").asc())
            .limit(limit)
            .select(id);

        crate::debug_sql(&query);

        let due: Vec<PaymentTransactionId> = query
            .get_results(conn)
            .await
            .attach("Error listing transactions due for reconciliation")
            .into_db_result()?;
        if due.is_empty() {
            return Ok(Vec::new());
        }

        diesel::update(payment_transaction.filter(id.eq_any(due)))
            .set(last_reconciled_at.eq(common_domain::clock::now()))
            .returning(PaymentTransactionRow::as_returning())
            .get_results(conn)
            .await
            .attach("Error claiming transactions for reconciliation")
            .into_db_result()
    }

    /// Invoice charges committed before sending whose provider answer never came back (no
    /// provider id yet), stamped like [`Self::claim_pending_for_reconcile`] so a row the
    /// provider keeps refusing is retried with the same backoff. Run it in a transaction: the
    /// row locks are what keep a concurrent sweeper off the same rows.
    pub async fn claim_unsent_charges(
        conn: &mut PgConn,
        older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            charge_attempt, created_at, id, last_reconciled_at, payment_transaction, payment_type,
            provider_transaction_id, status,
        };
        use diesel::dsl::sql;
        use diesel::sql_types::{Bool, Timestamptz};
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(status.eq(PaymentStatusEnum::Pending))
            .filter(payment_type.eq(PaymentTypeEnum::Payment))
            .filter(provider_transaction_id.is_null())
            .filter(charge_attempt.is_not_null())
            .filter(created_at.lt(older_than))
            .filter(sql::<Bool>(
                "(last_reconciled_at IS NULL OR last_reconciled_at < NOW() - \
                 LEAST(GREATEST((NOW() - created_at) / 10, INTERVAL '1 minute'), INTERVAL '1 hour'))",
            ))
            .order(sql::<Timestamptz>("COALESCE(last_reconciled_at, created_at)").asc())
            .limit(limit)
            .select(id)
            // A concurrent sweeper skips the rows this one is claiming instead of re-sending them.
            .for_update()
            .skip_locked();

        crate::debug_sql(&query);

        let due: Vec<PaymentTransactionId> = query
            .get_results(conn)
            .await
            .attach("Error listing unsent charges")
            .into_db_result()?;
        if due.is_empty() {
            return Ok(Vec::new());
        }

        diesel::update(payment_transaction.filter(id.eq_any(due)))
            .set(last_reconciled_at.eq(common_domain::clock::now()))
            .returning(PaymentTransactionRow::as_returning())
            .get_results(conn)
            .await
            .attach("Error claiming unsent charges")
            .into_db_result()
    }

    /// Removes a committed charge the provider refused before creating anything, as a
    /// single-transaction charge's rollback would have. Such a row was never ingested by
    /// analytics (no `processed_at`). A row an answer already moved on is kept.
    pub async fn delete_unsent_charge(
        conn: &mut PgConn,
        tx_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::payment_transaction::dsl::{
            id, payment_transaction, provider_transaction_id, status, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        diesel::delete(
            payment_transaction
                .filter(id.eq(tx_id))
                .filter(tenant_id.eq(tenant_uid))
                .filter(status.eq(PaymentStatusEnum::Pending))
                .filter(provider_transaction_id.is_null()),
        )
        .execute(conn)
        .await
        .attach("Error deleting unsent charge")
        .into_db_result()
    }

    /// Refunds we issued but never got an answer for: the provider call failed in transport, so
    /// the row never left `Ready` and carries no provider id. The money may or may not have
    /// moved, so the row is re-issued rather than abandoned.
    pub async fn list_stale_ready_refunds(
        conn: &mut PgConn,
        older_than: chrono::DateTime<chrono::Utc>,
        no_older_than: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl::{
            created_at, payment_transaction, payment_type, refund_mode, status,
        };
        use diesel_async::RunQueryDsl;

        let query = payment_transaction
            .filter(status.eq(PaymentStatusEnum::Ready))
            .filter(payment_type.eq(PaymentTypeEnum::Refund))
            .filter(refund_mode.eq(crate::enums::RefundModeEnum::Online))
            .filter(created_at.lt(older_than))
            // A replay is only safe while the provider still remembers the idempotency key.
            // Past that, a row that did go through would be paid a second time, so it stops
            // being swept and waits for a human instead.
            .filter(created_at.gt(no_older_than))
            .order(created_at.asc())
            .limit(limit)
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error listing stale ready refunds")
            .into_db_result()
    }

    /// REFUND rows issued against a settled payment, oldest first.
    pub async fn list_refunds_by_parent(
        conn: &mut PgConn,
        parent_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::parent_transaction_id.eq(parent_id))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .order(pt_dsl::created_at.asc())
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing refunds of a transaction")
            .into_db_result()
    }

    /// REFUND rows drawn against a credit note, oldest first.
    pub async fn list_refunds_by_credit_note(
        conn: &mut PgConn,
        credit_note_uid: CreditNoteId,
        tenant_uid: TenantId,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::credit_note_id.eq(credit_note_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .order(pt_dsl::created_at.asc())
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing refunds of a credit note")
            .into_db_result()
    }

    /// Σ amount of this payment's REFUND rows in `statuses`. Summed in Rust like the other
    /// money aggregates here, so the figure stays i64 minor units end to end.
    pub async fn sum_refunds_by_parent(
        conn: &mut PgConn,
        parent_id: PaymentTransactionId,
        tenant_uid: TenantId,
        statuses: &[PaymentStatusEnum],
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::parent_transaction_id.eq(parent_id))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq_any(statuses.to_vec()))
            .select(pt_dsl::amount);

        crate::debug_sql(&query);

        let amounts: Vec<i64> = query
            .load(conn)
            .await
            .attach("Error while summing refunds of a transaction")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Σ amount of this transaction's REFUND rows in `statuses` that went through the provider.
    /// The figure a provider reports as its cumulative refunded total covers only these, so an
    /// offline refund must never be counted against it.
    pub async fn sum_online_refunds_by_parent(
        conn: &mut PgConn,
        parent_id: PaymentTransactionId,
        tenant_uid: TenantId,
        statuses: &[PaymentStatusEnum],
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::parent_transaction_id.eq(parent_id))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq_any(statuses.to_vec()))
            .filter(pt_dsl::refund_mode.eq(crate::enums::RefundModeEnum::Online))
            .select(pt_dsl::amount);

        crate::debug_sql(&query);

        let amounts: Vec<i64> = query
            .load(conn)
            .await
            .attach("Error while summing online refunds of a transaction")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Σ amount of an invoice's settled REFUND rows that draw on a credit note. Their avoir
    /// already shrank the invoice total, so the payment they came out of no longer covers what
    /// it used to: netting them keeps the two sides of the balance in step. A refund with no
    /// credit note (a provider-side refund we only observed) is deliberately excluded — it
    /// leaves the invoice as it stands.
    pub async fn sum_credit_note_backed_refunds_by_invoice(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq(PaymentStatusEnum::Settled))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Refund))
            .filter(pt_dsl::credit_note_id.is_not_null())
            .select(pt_dsl::amount);

        crate::debug_sql(&query);

        let amounts: Vec<i64> = query
            .load(conn)
            .await
            .attach("Error while summing credit-note-backed refunds of an invoice")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Σ refunds of the payment not drawn on a credit note, issued, in flight or settled.
    pub async fn sum_live_refunds_without_note_by_parent(
        conn: &mut PgConn,
        parent_id: PaymentTransactionId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let amounts: Vec<i64> = pt_dsl::payment_transaction
            .filter(pt_dsl::parent_transaction_id.eq(parent_id))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq_any([
                PaymentStatusEnum::Ready,
                PaymentStatusEnum::Pending,
                PaymentStatusEnum::Settled,
            ]))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Refund))
            .filter(pt_dsl::credit_note_id.is_null())
            .select(pt_dsl::amount)
            .load(conn)
            .await
            .attach("Error while summing refunds of a payment without a credit note")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Σ refunds of the invoice not drawn on a credit note, issued (Ready), in flight (Pending)
    /// or settled: from its issue, a refund holds back what it gives.
    pub async fn sum_live_refunds_without_note_by_invoice(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq_any([
                PaymentStatusEnum::Ready,
                PaymentStatusEnum::Pending,
                PaymentStatusEnum::Settled,
            ]))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Refund))
            .filter(pt_dsl::credit_note_id.is_null())
            .select(pt_dsl::amount);

        crate::debug_sql(&query);

        let amounts: Vec<i64> = query
            .load(conn)
            .await
            .attach("Error while summing refunds of an invoice without a credit note")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Σ `refund_excess` of the invoice's refunds without a credit note that are issued, in flight
    /// or settled: the excess collections gave back while it was there.
    pub async fn sum_live_refund_excess_by_invoice(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let amounts: Vec<i64> = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq_any([
                PaymentStatusEnum::Ready,
                PaymentStatusEnum::Pending,
                PaymentStatusEnum::Settled,
            ]))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Refund))
            .filter(pt_dsl::credit_note_id.is_null())
            .select(pt_dsl::refund_excess)
            .load(conn)
            .await
            .attach("Error while summing the excess refunds of an invoice gave back")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// Records the part of a just-issued refund without a credit note that gave back excess.
    pub async fn set_refund_excess(
        conn: &mut PgConn,
        lock: &CustomerLock,
        refund: PaymentTransactionId,
        excess: i64,
    ) -> DbResult<()> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        diesel::update(pt_dsl::payment_transaction)
            .filter(pt_dsl::id.eq(refund))
            .filter(pt_dsl::tenant_id.eq(lock.tenant_id()))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Refund))
            .set(pt_dsl::refund_excess.eq(excess))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while recording the excess a refund gave back")
            .into_db_result()
    }

    /// Σ amount of this credit note's REFUND rows in `statuses` — what it has already drawn.
    pub async fn sum_refunds_by_credit_note(
        conn: &mut PgConn,
        credit_note_uid: CreditNoteId,
        tenant_uid: TenantId,
        statuses: &[PaymentStatusEnum],
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::credit_note_id.eq(credit_note_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::status.eq_any(statuses.to_vec()))
            .select(pt_dsl::amount);

        crate::debug_sql(&query);

        let amounts: Vec<i64> = query
            .load(conn)
            .await
            .attach("Error while summing refunds of a credit note")
            .into_db_result()?;
        Ok(amounts.iter().sum())
    }

    /// The REFUND rows of the provider's own refund id, in id order: one, or several when a
    /// refund observed at the provider was drawn on open refund notes in parts. Backs the
    /// observed-refund and `refund.updated` webhook paths, which only carry the provider id.
    pub async fn find_refund_by_provider_id(
        conn: &mut PgConn,
        lock: &CustomerLock,
        provider_refund_id: &str,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        let tenant_uid = lock.tenant_id();
        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::payment_type.eq(PaymentTypeEnum::Refund))
            .filter(pt_dsl::provider_transaction_id.eq(provider_refund_id))
            .order(pt_dsl::id.asc())
            .select(PaymentTransactionRow::as_select())
            // Callers hold the parent's lock and are about to patch these rows; reading them
            // unlocked lets a concurrent outcome be overwritten by a stale one.
            .for_update();

        crate::debug_sql(&query);

        let rows: Vec<PaymentTransactionRow> = query
            .load(conn)
            .await
            .attach("Error while finding refund by provider id")
            .into_db_result()?;
        for row in &rows {
            let owner = Self::customer_of(conn, row.tenant_id, row.id).await?;
            lock.covers(conn, row.tenant_id, owner)?;
        }
        Ok(rows)
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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while patching transaction")
            .into_db_result()
    }
}

impl PaymentTransactionRow {
    /// Cancel a transaction ONLY while still awaiting (Pending/Ready), in one
    /// status-predicated statement. 0 rows updated means a concurrent writer
    /// progressed the row first — the caller MUST treat that as "already
    /// progressed", never as a completed cancellation (an unguarded patch
    /// would clobber a concurrently-settled row).
    pub async fn cancel_if_awaiting(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        tx_id: PaymentTransactionId,
        error: &str,
    ) -> DbResult<usize> {
        use crate::schema::payment_transaction::dsl::{
            error_type, id, next_action, payment_transaction, pending_provider_intent_id, status,
            tenant_id,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::update(
            payment_transaction
                .filter(id.eq(tx_id))
                .filter(tenant_id.eq(tenant_uid))
                .filter(status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready])),
        )
        .set((
            status.eq(PaymentStatusEnum::Cancelled),
            error_type.eq(Some(error.to_string())),
            next_action.eq(None::<serde_json::Value>),
            // The hosted attempt is closed out with it — stop sweeping it.
            pending_provider_intent_id.eq(None::<String>),
        ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while cancelling awaiting transaction")
            .into_db_result()
    }

    /// The most recent Pending/Ready/Settled transaction for a checkout session, if
    /// any. Makes checkout completion idempotent: a re-invocation while a charge is
    /// in flight, or settled but not yet fulfilled, returns the existing transaction
    /// instead of issuing a fresh charge (a new idempotency key, so a double charge).
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
            .filter(status.eq_any([
                PaymentStatusEnum::Pending,
                PaymentStatusEnum::Ready,
                PaymentStatusEnum::Settled,
            ]))
            .order(created_at.desc())
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding active checkout transaction")
            .into_db_result()
    }

    pub async fn count_failed_by_checkout_session_id(
        conn: &mut PgConn,
        checkout_session_uid: CheckoutSessionId,
        tenant_uid: TenantId,
    ) -> DbResult<i64> {
        use crate::schema::payment_transaction::dsl::{
            checkout_session_id, payment_transaction, status, tenant_id,
        };
        use diesel_async::RunQueryDsl;

        payment_transaction
            .filter(checkout_session_id.eq(checkout_session_uid))
            .filter(tenant_id.eq(tenant_uid))
            .filter(status.eq(PaymentStatusEnum::Failed))
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting declined checkout transactions")
            .into_db_result()
    }

    /// The most recent transaction for a checkout session, at ANY status. Unlike
    /// [`Self::get_active_by_checkout_session_id`] (Pending/Ready/Settled), this also
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

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while tagging transaction with checkout session")
            .into_db_result()
    }

    /// Sweeper scan (all tenants): transactions still carrying a hosted
    /// pending-intent id, created before `older_than`. Failed/Cancelled rows
    /// stay watched (their hosted page can still capture on a retry) and
    /// Settled rows stay until the marker is released; only Refunded is
    /// excluded. Keyset-ordered on `(created_at, id)`; `after` pages onward.
    pub async fn list_sweepable_with_pending_intent(
        conn: &mut PgConn,
        older_than: chrono::DateTime<chrono::Utc>,
        after: Option<(chrono::DateTime<chrono::Utc>, PaymentTransactionId)>,
        limit: i64,
    ) -> DbResult<Vec<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel::BoolExpressionMethods;
        use diesel_async::RunQueryDsl;

        let mut query = pt_dsl::payment_transaction
            .filter(pt_dsl::pending_provider_intent_id.is_not_null())
            .filter(pt_dsl::status.eq_any([
                PaymentStatusEnum::Pending,
                PaymentStatusEnum::Ready,
                PaymentStatusEnum::Failed,
                PaymentStatusEnum::Cancelled,
                PaymentStatusEnum::Settled,
            ]))
            .filter(pt_dsl::created_at.lt(older_than))
            .order_by((pt_dsl::created_at.asc(), pt_dsl::id.asc()))
            .limit(limit)
            .select(PaymentTransactionRow::as_select())
            .into_boxed();

        if let Some((after_created_at, after_id)) = after {
            query = query.filter(
                pt_dsl::created_at
                    .gt(after_created_at)
                    .or(pt_dsl::created_at
                        .eq(after_created_at)
                        .and(pt_dsl::id.gt(after_id))),
            );
        }

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing transactions awaiting hosted completion")
            .into_db_result()
    }

    /// Clear the hosted pending-intent marker, but only while the row still
    /// carries exactly `intent_id`. 0 rows updated means the marker changed
    /// concurrently — the caller must not treat the intent as closed out.
    pub async fn clear_pending_intent_if_matches(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        tx_id: PaymentTransactionId,
        intent_id: &str,
    ) -> DbResult<usize> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(
            pt_dsl::payment_transaction
                .filter(pt_dsl::id.eq(tx_id))
                .filter(pt_dsl::tenant_id.eq(tenant_uid))
                .filter(pt_dsl::pending_provider_intent_id.eq(intent_id)),
        )
        .set(pt_dsl::pending_provider_intent_id.eq(None::<String>));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while clearing transaction pending intent")
            .into_db_result()
    }

    /// Clear the marker without the intent-id predicate — safe because the
    /// marker is write-once per row. Used under the row lock.
    pub async fn clear_pending_intent(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        tx_id: PaymentTransactionId,
    ) -> DbResult<usize> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(
            pt_dsl::payment_transaction
                .filter(pt_dsl::id.eq(tx_id))
                .filter(pt_dsl::tenant_id.eq(tenant_uid))
                .filter(pt_dsl::pending_provider_intent_id.is_not_null()),
        )
        .set(pt_dsl::pending_provider_intent_id.eq(None::<String>));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while clearing transaction pending intent")
            .into_db_result()
    }

    /// Most recent transaction for an invoice still carrying a hosted
    /// pending-intent id, at ANY status. Backs the single-intent discipline:
    /// re-initiation cancels (or adopts) this intent before minting a replacement.
    pub async fn latest_with_pending_intent_by_invoice_id(
        conn: &mut PgConn,
        inv_uid: InvoiceId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::invoice_id.eq(inv_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::pending_provider_intent_id.is_not_null())
            .order(pt_dsl::created_at.desc())
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding hosted invoice transaction")
            .into_db_result()
    }

    /// Most recent transaction for a checkout session still carrying a hosted
    /// pending-intent id, at ANY status — deliberately NOT "the latest
    /// transaction", since an intermediate saved-card attempt (no marker)
    /// must not hide a still-live prior capturable intent.
    pub async fn latest_with_pending_intent_by_checkout_session_id(
        conn: &mut PgConn,
        checkout_session_uid: CheckoutSessionId,
        tenant_uid: TenantId,
    ) -> DbResult<Option<PaymentTransactionRow>> {
        use crate::schema::payment_transaction::dsl as pt_dsl;
        use diesel_async::RunQueryDsl;

        let query = pt_dsl::payment_transaction
            .filter(pt_dsl::checkout_session_id.eq(checkout_session_uid))
            .filter(pt_dsl::tenant_id.eq(tenant_uid))
            .filter(pt_dsl::pending_provider_intent_id.is_not_null())
            .order(pt_dsl::created_at.desc())
            .select(PaymentTransactionRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding hosted checkout transaction")
            .into_db_result()
    }
}
