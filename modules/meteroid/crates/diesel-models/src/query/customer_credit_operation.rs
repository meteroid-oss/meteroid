//! Top-up purchases of customer credit accounts.
use crate::customer_credit::{
    CustomerCreditAccountRow, CustomerCreditOperationRow, CustomerCreditOperationRowNew,
};
use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    CreditOperationId, CustomerId, InvoiceId, PaymentTransactionId, TenantId, WalletId,
};
use diesel::{
    ExpressionMethods, OptionalExtension, PgJsonbExpressionMethods, QueryDsl, SelectableHelper,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use rust_decimal::Decimal;

/// Operation states, as stored.
pub mod state {
    pub const CREATED: &str = "CREATED";
    pub const PROCESSING: &str = "PROCESSING";
    pub const SETTLED: &str = "SETTLED";
    pub const FAILED: &str = "FAILED";
    pub const CANCELLED: &str = "CANCELLED";
}

impl CustomerCreditOperationRowNew {
    /// None when an operation with the same identity already exists: the same idempotency key of
    /// the customer, or a top-up already in flight on the account.
    pub async fn insert_conflict_do_nothing(
        &self,
        conn: &mut PgConn,
    ) -> DbResult<Option<CustomerCreditOperationRow>> {
        use crate::schema::customer_credit_operation::dsl as o;
        diesel::insert_into(o::customer_credit_operation)
            .values((
                self,
                crate::schema::customer_credit_operation::created_at
                    .eq(common_domain::clock::now()),
            ))
            .on_conflict_do_nothing()
            .returning(CustomerCreditOperationRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while inserting a customer credit operation")
            .into_db_result()
    }
}

impl CustomerCreditOperationRow {
    /// Metadata key listing the charges earlier attempts left behind when the key was re-armed.
    pub const SUPERSEDED_PAYMENTS_KEY: &'static str = "superseded_payment_transaction_ids";
    /// The metadata counter of the charges an operation attempted, resumptions included.
    pub const ATTEMPTS_KEY: &'static str = "attempts";

    pub async fn lock_by_id(
        conn: &mut PgConn,
        lock: &CustomerLock,
        id: CreditOperationId,
    ) -> DbResult<Self> {
        use crate::schema::customer_credit_operation::dsl as o;
        lock.on(conn);
        let (tenant_id, customer_id) = (lock.tenant_id(), lock.customer_id());
        o::customer_credit_operation
            .filter(o::tenant_id.eq(tenant_id))
            .filter(o::customer_id.eq(customer_id))
            .filter(o::id.eq(id))
            .for_no_key_update()
            .select(Self::as_select())
            .first(conn)
            .await
            .attach("Error while locking a customer credit operation")
            .into_db_result()
    }

    /// Keys are scoped to their customer: another customer's key never resolves here.
    pub async fn find_idempotent(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        namespace: &str,
        idempotency_key: &str,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        o::customer_credit_operation
            .filter(o::tenant_id.eq(tenant_id))
            .filter(o::customer_id.eq(customer_id))
            .filter(o::namespace.eq(namespace))
            .filter(o::idempotency_key.eq(idempotency_key))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding an idempotent customer credit operation")
            .into_db_result()
    }

    /// Keys are scoped to their customer: another customer's key never resolves here.
    pub async fn find_idempotent_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        namespace: &str,
        idempotency_key: &str,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        lock.on(conn);
        let (tenant_id, customer_id) = (lock.tenant_id(), lock.customer_id());
        o::customer_credit_operation
            .filter(o::tenant_id.eq(tenant_id))
            .filter(o::customer_id.eq(customer_id))
            .filter(o::namespace.eq(namespace))
            .filter(o::idempotency_key.eq(idempotency_key))
            .for_no_key_update()
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding an idempotent customer credit operation")
            .into_db_result()
    }

    /// Operations the customer opened in the window, whatever their outcome.
    pub async fn count_since(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        namespace: &str,
        since: DateTime<Utc>,
    ) -> DbResult<i64> {
        use crate::schema::customer_credit_operation::dsl as o;
        o::customer_credit_operation
            .filter(o::tenant_id.eq(tenant_id))
            .filter(o::customer_id.eq(customer_id))
            .filter(o::namespace.eq(namespace))
            .filter(o::created_at.ge(since))
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting recent top-ups")
            .into_db_result()
    }

    /// Self-service top-up charges the customer started since `since`: each attempt is its own
    /// payment on its top-up checkout, dated when it was created. Resuming an attempt reuses its
    /// payment, so it counts once.
    pub async fn count_attempts_since(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        since: DateTime<Utc>,
    ) -> DbResult<i64> {
        use crate::enums::PaymentTypeEnum;
        use crate::schema::checkout_session::dsl as cs;
        use crate::schema::payment_transaction::dsl as pt;
        use diesel::{JoinOnDsl, NullableExpressionMethods};
        pt::payment_transaction
            .inner_join(cs::checkout_session.on(pt::checkout_session_id.eq(cs::id.nullable())))
            .filter(cs::tenant_id.eq(tenant_id))
            .filter(cs::customer_id.eq(customer_id))
            .filter(pt::tenant_id.eq(tenant_id))
            .filter(pt::payment_type.eq(PaymentTypeEnum::TopUp))
            .filter(pt::created_at.ge(since))
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting recent top-up attempts")
            .into_db_result()
    }

    /// Automatic top-ups not yet armed, oldest first. Each is locked and re-checked when armed.
    pub async fn list_to_arm(
        conn: &mut PgConn,
        namespace: &str,
        limit: i64,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        o::customer_credit_operation
            .filter(o::state.eq(state::CREATED))
            .filter(o::namespace.eq(namespace))
            .order(o::created_at.asc())
            .limit(limit)
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing automatic top-ups to arm")
            .into_db_result()
    }

    pub async fn find_by_payment_transaction(
        conn: &mut PgConn,
        tenant_id: TenantId,
        payment_transaction_id: PaymentTransactionId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        o::customer_credit_operation
            .filter(o::tenant_id.eq(tenant_id))
            .filter(o::payment_transaction_id.eq(payment_transaction_id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding a top-up by payment")
            .into_db_result()
    }

    pub async fn find_by_payment_transaction_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        payment_transaction_id: PaymentTransactionId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        lock.on(conn);
        o::customer_credit_operation
            .filter(o::tenant_id.eq(lock.tenant_id()))
            .filter(o::customer_id.eq(lock.customer_id()))
            .filter(o::payment_transaction_id.eq(payment_transaction_id))
            .for_no_key_update()
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while locking a top-up by payment")
            .into_db_result()
    }

    /// The settled top-up whose credit an invoice sold.
    pub async fn find_settled_by_invoice_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice_id: InvoiceId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        lock.on(conn);
        o::customer_credit_operation
            .filter(o::tenant_id.eq(lock.tenant_id()))
            .filter(o::customer_id.eq(lock.customer_id()))
            .filter(o::invoice_id.eq(invoice_id))
            .filter(o::state.eq(state::SETTLED))
            .order(o::created_at.desc())
            .for_no_key_update()
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while locking a top-up by invoice")
            .into_db_result()
    }

    /// When the earliest operation of `namespace` billed by `invoice_id` was created (queued).
    pub async fn created_at_for_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        namespace: &str,
    ) -> DbResult<Option<DateTime<Utc>>> {
        use crate::schema::customer_credit_operation::dsl as o;
        use diesel::dsl::min;
        o::customer_credit_operation
            .filter(o::tenant_id.eq(tenant_id))
            .filter(o::invoice_id.eq(invoice_id))
            .filter(o::namespace.eq(namespace))
            .select(min(o::created_at))
            .first::<Option<DateTime<Utc>>>(conn)
            .await
            .attach("Error while reading when a top-up was queued")
            .into_db_result()
    }

    pub async fn find_by_superseded_payment(
        conn: &mut PgConn,
        tenant_id: TenantId,
        payment_transaction_id: PaymentTransactionId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        o::customer_credit_operation
            .filter(o::tenant_id.eq(tenant_id))
            .filter(o::metadata.contains(serde_json::json!({
                Self::SUPERSEDED_PAYMENTS_KEY: [payment_transaction_id.to_string()]
            })))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while finding a top-up by a superseded payment")
            .into_db_result()
    }

    /// Attempts left `PROCESSING` whose outcome never reached the database. Unlocked: each is
    /// locked and re-checked on its own, behind its payment and customer.
    pub async fn list_stuck(
        conn: &mut PgConn,
        namespaces: &[&str],
        updated_before: DateTime<Utc>,
        limit: i64,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        o::customer_credit_operation
            .filter(o::state.eq(state::PROCESSING))
            .filter(o::namespace.eq_any(namespaces))
            .filter(o::updated_at.le(updated_before))
            .order(o::updated_at.asc())
            .limit(limit)
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing stuck top-ups")
            .into_db_result()
    }

    /// Binds one charge attempt: its payment (the provider identity), its draft purchase document
    /// and its price. Also used to retry a declined attempt, so it clears the previous outcome.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_attempt(
        conn: &mut PgConn,
        id: CreditOperationId,
        payment_transaction_id: PaymentTransactionId,
        invoice_id: InvoiceId,
        expected_consideration_minor: i64,
        consideration_currency: &str,
        metadata: serde_json::Value,
    ) -> DbResult<Self> {
        use crate::schema::customer_credit_operation::dsl as o;
        diesel::update(o::customer_credit_operation.filter(o::id.eq(id)))
            .set((
                o::state.eq(state::PROCESSING),
                o::payment_transaction_id.eq(Some(payment_transaction_id)),
                o::invoice_id.eq(Some(invoice_id)),
                o::expected_consideration_minor.eq(Some(expected_consideration_minor)),
                o::consideration_currency.eq(Some(consideration_currency.to_owned())),
                o::metadata.eq(metadata),
                o::updated_at.eq(common_domain::clock::now().naive_utc()),
            ))
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while starting a top-up attempt")
            .into_db_result()
    }

    /// Records progress on an attempt still in flight; a finished one is left as it is.
    pub async fn touch(
        conn: &mut PgConn,
        id: CreditOperationId,
        metadata: serde_json::Value,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_operation::dsl as o;
        diesel::update(
            o::customer_credit_operation
                .filter(o::id.eq(id))
                .filter(o::state.eq(state::PROCESSING)),
        )
        .set((
            o::metadata.eq(metadata),
            o::updated_at.eq(common_domain::clock::now().naive_utc()),
        ))
        .returning(Self::as_returning())
        .get_result(conn)
        .await
        .optional()
        .attach("Error while recording top-up progress")
        .into_db_result()
    }

    /// Withdraws the account's queued operations of `namespace` that never armed a charge.
    pub async fn cancel_queued(
        conn: &mut PgConn,
        account_id: WalletId,
        namespace: &str,
    ) -> DbResult<usize> {
        use crate::schema::customer_credit_operation::dsl as o;
        diesel::update(
            o::customer_credit_operation
                .filter(o::account_id.eq(account_id))
                .filter(o::namespace.eq(namespace))
                .filter(o::state.eq(state::CREATED))
                .filter(o::payment_transaction_id.is_null()),
        )
        .set((
            o::state.eq(state::CANCELLED),
            o::updated_at.eq(common_domain::clock::now().naive_utc()),
        ))
        .execute(conn)
        .await
        .attach("Error while withdrawing queued top-ups")
        .into_db_result()
    }

    /// Settles the operation as of `settled_at`, the moment its payment was captured, which the
    /// spending caps and the cooldown read.
    pub async fn settle_at(
        conn: &mut PgConn,
        id: CreditOperationId,
        metadata: serde_json::Value,
        settled_at: DateTime<Utc>,
    ) -> DbResult<Self> {
        use crate::schema::customer_credit_operation::dsl as o;
        diesel::update(o::customer_credit_operation.filter(o::id.eq(id)))
            .set((
                o::state.eq(state::SETTLED),
                o::metadata.eq(metadata),
                o::updated_at.eq(common_domain::clock::now().naive_utc()),
                o::settled_at.eq(Some(settled_at)),
            ))
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while settling a top-up")
            .into_db_result()
    }

    pub async fn finish(
        conn: &mut PgConn,
        id: CreditOperationId,
        state: &str,
        metadata: serde_json::Value,
    ) -> DbResult<Self> {
        use crate::schema::customer_credit_operation::dsl as o;
        let target = diesel::update(o::customer_credit_operation.filter(o::id.eq(id)));
        if state == self::state::SETTLED {
            return target
                .set((
                    o::state.eq(state),
                    o::metadata.eq(metadata),
                    o::updated_at.eq(common_domain::clock::now().naive_utc()),
                    o::settled_at.eq(common_domain::clock::now().naive_utc()),
                ))
                .returning(Self::as_returning())
                .get_result(conn)
                .await
                .attach("Error while finishing a top-up")
                .into_db_result();
        }
        target
            .set((
                o::state.eq(state),
                o::metadata.eq(metadata),
                o::updated_at.eq(common_domain::clock::now().naive_utc()),
            ))
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while finishing a top-up")
            .into_db_result()
    }
}

/// A wallet rule changes what the live projection computes: the customer is recomputed at once.
async fn mark_configured(conn: &mut PgConn, account: &CustomerCreditAccountRow) -> DbResult<()> {
    crate::customer_credit::CreditRecomputeMarkRow::mark_one(
        conn,
        account.tenant_id,
        account.customer_id,
    )
    .await
    .map(|_| ())
}

impl CustomerCreditAccountRow {
    /// The customer's agreed price per credit in `currency`; None clears it.
    pub async fn set_unit_price(
        conn: &mut PgConn,
        lock: &CustomerLock,
        id: WalletId,
        price: Option<(Decimal, String)>,
    ) -> DbResult<Self> {
        use crate::schema::customer_credit_account::dsl as a;
        lock.on(conn);
        let (unit_price, currency) = price.unzip();
        let row = diesel::update(
            a::customer_credit_account
                .filter(a::tenant_id.eq(lock.tenant_id()))
                .filter(a::customer_id.eq(lock.customer_id()))
                .filter(a::id.eq(id)),
        )
        .set((
            a::unit_price.eq(unit_price),
            a::unit_price_currency.eq(currency),
        ))
        .returning(Self::as_returning())
        .get_result(conn)
        .await
        .attach("Error while setting a wallet's agreed unit price")
        .into_db_result()?;
        mark_configured(conn, &row).await?;
        Ok(row)
    }
}
