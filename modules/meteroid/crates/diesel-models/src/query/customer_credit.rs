use crate::customer_credit::{
    CreditLineAllocationRowNew, CreditPurchaseRow, CreditPurchaseRowNew, CreditUnrecoveredRow,
    CreditUsageSettlementRow, CustomerCreditAccountRow, CustomerCreditAccountRowNew,
    CustomerCreditEntryRow, CustomerCreditEntryRowNew, CustomerCreditGrantProductRow,
    CustomerCreditGrantRow, CustomerCreditGrantRowNew, CustomerCreditGrantStateFilter,
    CustomerCreditRosterRow, CustomerCreditStatementRow, DueExpiryCandidate,
    EndedSubscriptionCredit, GrantRemaining, GrantSubscriptionFilter, HistoricalBalanceTxRow,
    PlanCreditRuleRow, ScopedGrantRow, SubscriptionFilter,
};
use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CreditNoteId, CustomerId, InvoiceId, ProductId, SubscriptionId,
    TenantId, WalletId,
};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, OptionalExtension, QueryDsl,
    SelectableHelper, sql_types,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

/// Grants of one customer joined to their account, filtered by currency, by the subscription
/// they belong to and by state at `now`; the states mirror how a grant is presented (lapsed, then
/// pending, then depleted or active).
macro_rules! grant_page_query {
    ($tenant_id:expr, $customer_id:expr, $currency:expr, $subscription:expr, $state:expr, $now:expr) => {{
        let now: DateTime<Utc> = $now;
        let mut query = g::customer_credit_grant
            .inner_join(a::customer_credit_account.on(a::id.eq(g::account_id)))
            .filter(g::tenant_id.eq($tenant_id))
            .filter(g::customer_id.eq($customer_id))
            .into_boxed();
        if let Some(currency) = $currency {
            query = query.filter(a::currency.eq(currency.to_owned()));
        }
        match $subscription {
            None => {}
            Some(GrantSubscriptionFilter::Of(subscription_id)) => {
                query = query.filter(g::subscription_id.eq(subscription_id));
            }
            Some(GrantSubscriptionFilter::UsableBy(subscription_id)) => {
                query = query.filter(
                    g::subscription_id
                        .is_null()
                        .or(g::subscription_id.eq(subscription_id)),
                );
            }
        }
        match $state {
            None => query,
            Some(CustomerCreditGrantStateFilter::Expired) => query.filter(g::lapses_at.le(now)),
            Some(CustomerCreditGrantStateFilter::Pending) => query
                .filter(g::lapses_at.is_null().or(g::lapses_at.gt(now)))
                .filter(g::effective_at.gt(now)),
            Some(CustomerCreditGrantStateFilter::Depleted) => query
                .filter(g::lapses_at.is_null().or(g::lapses_at.gt(now)))
                .filter(g::effective_at.le(now))
                .filter(g::remaining.eq(0)),
            Some(CustomerCreditGrantStateFilter::Active) => query
                .filter(g::lapses_at.is_null().or(g::lapses_at.gt(now)))
                .filter(g::effective_at.le(now))
                .filter(g::remaining.gt(0)),
            Some(CustomerCreditGrantStateFilter::UsedUp) => query.filter(
                g::effective_at
                    .le(now)
                    .and(g::remaining.eq(0))
                    .or(g::lapses_at.le(now)),
            ),
        }
    }};
}

/// Each grant matching `{filter}` (on `g`) with its products, grouped once in a CTE over the same
/// grants (`{products}` on `c`), in id order; `{lock}` locks the grant rows only.
fn scoped_grants_query(products: &str, filter: &str, lock: &str) -> String {
    format!(
        "WITH gp AS (
             SELECT p.grant_id, array_agg(p.product_id ORDER BY p.product_id) AS product_ids
             FROM customer_credit_grant_product p
             JOIN customer_credit_grant c ON c.id = p.grant_id
             WHERE p.tenant_id = $1 AND {products}
             GROUP BY p.grant_id)
         SELECT g.*, coalesce(gp.product_ids, '{{}}'::uuid[]) AS product_ids
         FROM customer_credit_grant g
         LEFT JOIN gp ON gp.grant_id = g.id
         WHERE {filter}
         ORDER BY g.id
         {lock}"
    )
}

impl CustomerCreditAccountRow {
    pub async fn find_or_create(
        conn: &mut PgConn,
        row: &CustomerCreditAccountRowNew,
    ) -> DbResult<Self> {
        Ok(Self::find_or_create_reporting(conn, row).await?.0)
    }

    /// `find_or_create`, also telling whether the wallet was created.
    pub async fn find_or_create_reporting(
        conn: &mut PgConn,
        row: &CustomerCreditAccountRowNew,
    ) -> DbResult<(Self, bool)> {
        use crate::schema::customer_credit_account::dsl as a;
        let created = diesel::insert_into(a::customer_credit_account)
            .values(row)
            .on_conflict((a::tenant_id, a::customer_id, a::currency))
            .do_nothing()
            .execute(conn)
            .await
            .attach("Error while creating a customer credit account")
            .into_db_result()?;
        // A first wallet brings the customer into the live projection at once.
        if created > 0 {
            crate::customer_credit::CreditRecomputeMarkRow::mark_one(
                conn,
                row.tenant_id,
                row.customer_id,
            )
            .await?;
        }
        a::customer_credit_account
            .filter(a::tenant_id.eq(row.tenant_id))
            .filter(a::customer_id.eq(row.customer_id))
            .filter(a::currency.eq(&row.currency))
            .select(Self::as_select())
            .get_result(conn)
            .await
            .attach("Error while reading a customer credit account")
            .into_db_result()
            .map(|account| (account, created > 0))
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: WalletId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_account::dsl as a;
        a::customer_credit_account
            .filter(a::tenant_id.eq(tenant_id))
            .filter(a::id.eq(id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a customer credit account")
            .into_db_result()
    }

    /// The customer's one wallet of a denomination (an ISO or credit unit code), if opened.
    pub async fn find_by_currency(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: &str,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_account::dsl as a;
        a::customer_credit_account
            .filter(a::tenant_id.eq(tenant_id))
            .filter(a::customer_id.eq(customer_id))
            .filter(a::currency.eq(currency))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a customer credit account")
            .into_db_result()
    }

    /// The customer's account. `NO KEY UPDATE`, so a grant inserted for it (`KEY SHARE`) is never
    /// blocked; a ledger movement's `bump_ledger_version` (an update of the row) waits for it.
    pub async fn lock_by_id(
        conn: &mut PgConn,
        lock: &CustomerLock,
        id: WalletId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_account::dsl as a;
        lock.on(conn);
        a::customer_credit_account
            .filter(a::tenant_id.eq(lock.tenant_id()))
            .filter(a::customer_id.eq(lock.customer_id()))
            .filter(a::id.eq(id))
            .for_no_key_update()
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while locking a customer credit account")
            .into_db_result()
    }

    pub async fn list_by_ids(conn: &mut PgConn, ids: &[WalletId]) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_account::dsl as a;
        a::customer_credit_account
            .filter(a::id.eq_any(ids))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading customer credit accounts")
            .into_db_result()
    }

    pub async fn list_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_account::dsl as a;
        a::customer_credit_account
            .filter(a::tenant_id.eq(tenant_id))
            .filter(a::customer_id.eq(customer_id))
            .order(a::currency.asc())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing customer credit accounts")
            .into_db_result()
    }
}

impl CustomerCreditGrantProductRow {
    pub async fn insert_all(conn: &mut PgConn, rows: &[Self]) -> DbResult<()> {
        use crate::schema::customer_credit_grant_product::dsl as p;
        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(p::customer_credit_grant_product)
            .values(rows)
            .execute(conn)
            .await
            .attach("Error while recording the products of a credit grant")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn list_by_grants(
        conn: &mut PgConn,
        tenant_id: TenantId,
        grant_ids: &[CreditGrantId],
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant_product::dsl as p;
        if grant_ids.is_empty() {
            return Ok(Vec::new());
        }
        p::customer_credit_grant_product
            .filter(p::tenant_id.eq(tenant_id))
            .filter(p::grant_id.eq_any(grant_ids))
            .order((p::grant_id.asc(), p::product_id.asc()))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading the products of credit grants")
            .into_db_result()
    }

    /// Of `product_ids`, those that are products of the tenant.
    pub async fn existing_products(
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_ids: &[ProductId],
    ) -> DbResult<Vec<ProductId>> {
        use crate::schema::product::dsl as pr;
        if product_ids.is_empty() {
            return Ok(Vec::new());
        }
        pr::product
            .filter(pr::tenant_id.eq(tenant_id))
            .filter(pr::id.eq_any(product_ids))
            .select(pr::id)
            .load(conn)
            .await
            .attach("Error while reading products")
            .into_db_result()
    }
}

impl CustomerCreditGrantRow {
    /// The debt-cancelling notes of invoices whose purchased paid credit still holds value past
    /// its lapse: forgiveness funded it, and a late pack credits it back before the sweep lapses it.
    pub async fn list_forgiven_purchase_notes(
        conn: &mut PgConn,
        lapsed_by: DateTime<Utc>,
        limit: i64,
    ) -> DbResult<Vec<(TenantId, CreditNoteId)>> {
        #[derive(diesel::QueryableByName)]
        struct Note {
            #[diesel(sql_type = sql_types::Uuid)]
            tenant_id: TenantId,
            #[diesel(sql_type = sql_types::Uuid)]
            credit_note_id: CreditNoteId,
        }
        diesel::sql_query(
            "SELECT DISTINCT n.tenant_id, n.id AS credit_note_id
             FROM customer_credit_grant g
             JOIN credit_note n ON n.tenant_id = g.tenant_id AND n.invoice_id = g.invoice_id
             WHERE g.remaining > 0 AND g.lapses_at <= $1
               AND g.source_kind = 'CREDIT_PURCHASE' AND g.classification = 'PAID'
               AND n.status = 'FINALIZED' AND n.debt_cancelled_amount_cents > 0
             LIMIT $2",
        )
        .bind::<sql_types::Timestamptz, _>(lapsed_by)
        .bind::<sql_types::BigInt, _>(limit)
        .get_results::<Note>(conn)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.tenant_id, row.credit_note_id))
                .collect()
        })
        .attach("Error while listing forgiven purchases")
        .into_db_result()
    }

    /// None when a grant already exists for this source.
    pub async fn insert_new(
        conn: &mut PgConn,
        row: &CustomerCreditGrantRowNew,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        diesel::insert_into(g::customer_credit_grant)
            .values(row)
            .on_conflict((g::tenant_id, g::source_kind, g::source_id))
            .do_nothing()
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while inserting a customer credit grant")
            .into_db_result()
    }

    /// Which of `source_ids` already funded a grant of `source_kind`.
    pub async fn list_funded_sources(
        conn: &mut PgConn,
        tenant_id: TenantId,
        source_kind: &str,
        source_ids: &[Uuid],
    ) -> DbResult<Vec<Uuid>> {
        use crate::schema::customer_credit_grant::dsl as g;
        if source_ids.is_empty() {
            return Ok(Vec::new());
        }
        g::customer_credit_grant
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::source_kind.eq(source_kind))
            .filter(g::source_id.eq_any(source_ids))
            .select(g::source_id)
            .distinct()
            .load(conn)
            .await
            .attach("Error while reading funded credit sources")
            .into_db_result()
    }

    pub async fn find_by_source(
        conn: &mut PgConn,
        tenant_id: TenantId,
        source_kind: &str,
        source_id: Uuid,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        g::customer_credit_grant
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::source_kind.eq(source_kind))
            .filter(g::source_id.eq(source_id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a customer credit grant")
            .into_db_result()
    }

    /// Grants a customer's invoice (or its credit notes) funded, locked in id order.
    pub async fn lock_by_invoice(
        conn: &mut PgConn,
        lock: &CustomerLock,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        let rows: Vec<Self> = g::customer_credit_grant
            .filter(g::tenant_id.eq(lock.tenant_id()))
            .filter(g::invoice_id.eq(invoice_id))
            .order(g::id.asc())
            .for_no_key_update()
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while locking invoice credit grants")
            .into_db_result()?;
        lock.on(conn);
        rows.iter()
            .try_for_each(|row| lock.covers(conn, row.tenant_id, row.customer_id))?;
        Ok(rows)
    }

    /// The customer's grants among `ids`. NO KEY UPDATE, as every grant lock: a grant is never
    /// deleted, and the movements and products inserted against it (KEY SHARE) must not wait on it.
    pub async fn lock_by_ids(
        conn: &mut PgConn,
        lock: &CustomerLock,
        ids: &[CreditGrantId],
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        lock.on(conn);
        g::customer_credit_grant
            .filter(g::tenant_id.eq(lock.tenant_id()))
            .filter(g::customer_id.eq(lock.customer_id()))
            .filter(g::id.eq_any(ids))
            .order(g::id.asc())
            .for_no_key_update()
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while locking customer credit grants")
            .into_db_result()
    }

    /// Grants by id with their products, spent or not; nothing is locked.
    pub async fn list_by_ids_scoped(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[CreditGrantId],
    ) -> DbResult<Vec<ScopedGrantRow>> {
        use diesel::sql_types::{Array, Uuid as SqlUuid};
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>();
        diesel::sql_query(scoped_grants_query(
            "c.tenant_id = $1 AND c.id = ANY($2)",
            "g.tenant_id = $1 AND g.id = ANY($2)",
            "",
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(ids)
        .load(conn)
        .await
        .attach("Error while reading customer credit grants")
        .into_db_result()
    }

    /// Unspent grants of one wallet a draw may take, with their products, locked in id order (a
    /// subset of `lock_all`'s order, so concurrent drawers never deadlock); the account row stays
    /// unlocked. Lapses are left to the allocator, which sees the same clock as its caller.
    pub async fn lock_drawable(
        conn: &mut PgConn,
        lock: &CustomerLock,
        account_id: WalletId,
        subscriptions: SubscriptionFilter,
    ) -> DbResult<Vec<ScopedGrantRow>> {
        Self::drawable(
            conn,
            lock.tenant_id(),
            lock.customer_id(),
            account_id,
            subscriptions,
            Some(lock),
        )
        .await
    }

    /// `lock_drawable` without locking anything, for estimates.
    pub async fn list_drawable(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        account_id: WalletId,
        subscriptions: SubscriptionFilter,
    ) -> DbResult<Vec<ScopedGrantRow>> {
        Self::drawable(
            conn,
            tenant_id,
            customer_id,
            account_id,
            subscriptions,
            None,
        )
        .await
    }

    async fn drawable(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        account_id: WalletId,
        subscriptions: SubscriptionFilter,
        lock: Option<&CustomerLock>,
    ) -> DbResult<Vec<ScopedGrantRow>> {
        use diesel::sql_types::{Array, Nullable, Uuid as SqlUuid};
        let subscriptions = match subscriptions {
            SubscriptionFilter::Any => None,
            SubscriptionFilter::CustomerWideOr(ids) => {
                Some(ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>())
            }
        };
        diesel::sql_query(scoped_grants_query(
            "c.tenant_id = $1 AND c.customer_id = $2 AND c.account_id = $3 AND c.remaining > 0",
            "g.tenant_id = $1 AND g.customer_id = $2 AND g.account_id = $3 AND g.remaining > 0
               AND ($4::uuid[] IS NULL OR g.subscription_id IS NULL OR g.subscription_id = ANY($4))",
            match lock {
                Some(lock) => {
                    lock.covers(conn, tenant_id, customer_id)?;
                    "FOR NO KEY UPDATE OF g"
                }
                None => "",
            },
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<SqlUuid, _>(customer_id.as_uuid())
        .bind::<SqlUuid, _>(account_id.as_uuid())
        .bind::<Nullable<Array<SqlUuid>>, _>(subscriptions)
        .load(conn)
        .await
        .attach("Error while reading the drawable credit of a customer")
        .into_db_result()
    }

    /// The denominations of the subscription's grants not ended yet.
    pub async fn unended_denominations_of_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<String>> {
        use crate::schema::customer_credit_account::dsl as a;
        use crate::schema::customer_credit_grant::dsl as g;
        g::customer_credit_grant
            .inner_join(a::customer_credit_account.on(a::id.eq(g::account_id)))
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::subscription_id.eq(subscription_id))
            .filter(g::ends_at.is_null())
            .select(a::currency)
            .distinct()
            .load(conn)
            .await
            .attach("Error while listing the denominations of a subscription's credit")
            .into_db_result()
    }

    /// Ends every grant of `subscription_id` not expired before `ended_at`, once: spent ones too,
    /// so value a later reversal gives back never becomes spendable again. A grant that expired
    /// first keeps its expiry as the reason it lapsed.
    pub async fn end_for_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        ended_at: DateTime<Utc>,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        diesel::update(
            g::customer_credit_grant
                .filter(g::tenant_id.eq(tenant_id))
                .filter(g::subscription_id.eq(subscription_id))
                .filter(g::ends_at.is_null())
                .filter(g::expires_at.is_null().or(g::expires_at.ge(ended_at))),
        )
        .set((
            g::ends_at.eq(ended_at),
            g::end_reason.eq(SUBSCRIPTION_ENDED),
        ))
        .returning(Self::as_returning())
        .get_results(conn)
        .await
        .attach("Error while ending the credit of a subscription")
        .into_db_result()
    }

    /// Ends the grants an immediate plan change removes at `ended_at`, once; spent ones too, so value
    /// given back later never becomes spendable again.
    pub async fn end_for_plan_change(
        conn: &mut PgConn,
        tenant_id: TenantId,
        grant_ids: &[CreditGrantId],
        ended_at: DateTime<Utc>,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        if grant_ids.is_empty() {
            return Ok(Vec::new());
        }
        diesel::update(
            g::customer_credit_grant
                .filter(g::tenant_id.eq(tenant_id))
                .filter(g::id.eq_any(grant_ids))
                .filter(g::subscription_id.is_not_null())
                .filter(g::ends_at.is_null()),
        )
        .set((g::ends_at.eq(ended_at), g::end_reason.eq(PLAN_CHANGE_ENDED)))
        .returning(Self::as_returning())
        .get_results(conn)
        .await
        .attach("Error while ending the credit a plan change removes")
        .into_db_result()
    }

    /// Terminal subscriptions with a grant `end_for_subscription` would still end, at most `limit`.
    /// Their end is the services' `credit_ended_at`: the termination instant, else the start of the
    /// period the terminal transition opened, never later than now.
    pub async fn list_unended_of_ended_subscriptions(
        conn: &mut PgConn,
        after: Option<(TenantId, SubscriptionId)>,
        limit: i64,
    ) -> DbResult<Vec<EndedSubscriptionCredit>> {
        let (after_tenant, after_subscription) = after.unzip();
        diesel::sql_query(
            "SELECT DISTINCT g.tenant_id, g.subscription_id
             FROM customer_credit_grant g
             JOIN subscription s ON s.id = g.subscription_id AND s.tenant_id = g.tenant_id
             WHERE g.ends_at IS NULL
               AND s.status IN ('CANCELLED', 'COMPLETED', 'ABORTED', 'SUPERSEDED')
               AND (g.expires_at IS NULL
                    OR g.expires_at >= LEAST(
                        COALESCE(s.terminated_at, s.current_period_start::timestamp)
                            AT TIME ZONE 'UTC',
                        now()))
               AND ($2::uuid IS NULL OR (g.tenant_id, g.subscription_id) > ($2, $3))
             ORDER BY g.tenant_id, g.subscription_id
             LIMIT $1",
        )
        .bind::<sql_types::BigInt, _>(limit)
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(after_tenant)
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(after_subscription)
        .load(conn)
        .await
        .attach("Error while listing the credit of ended subscriptions")
        .into_db_result()
    }

    /// Unspent value of each effective grant of one account that has not lapsed at `now`.
    pub async fn available_amounts_for_account(
        conn: &mut PgConn,
        account_id: WalletId,
        now: DateTime<Utc>,
    ) -> DbResult<Vec<i64>> {
        use crate::schema::customer_credit_grant::dsl as g;
        g::customer_credit_grant
            .filter(g::account_id.eq(account_id))
            .filter(g::remaining.gt(0))
            .filter(g::effective_at.le(now))
            .filter(g::lapses_at.is_null().or(g::lapses_at.gt(now)))
            .select(g::remaining)
            .load(conn)
            .await
            .attach("Error while reading available customer credit")
            .into_db_result()
    }

    /// The spendable grants of one account at `now` that pay any charge of any of the customer's
    /// subscriptions: customer-wide, not usage-only and not restricted to products.
    pub async fn shared_available_for_account(
        conn: &mut PgConn,
        account_id: WalletId,
        now: DateTime<Utc>,
    ) -> DbResult<Vec<GrantRemaining>> {
        use diesel::sql_types::{Timestamptz, Uuid as SqlUuid};
        diesel::sql_query(
            "WITH restricted AS (
                 SELECT DISTINCT p.grant_id
                 FROM customer_credit_grant_product p
                 JOIN customer_credit_grant c ON c.id = p.grant_id
                 WHERE c.account_id = $1 AND c.remaining > 0)
             SELECT g.id, g.remaining
             FROM customer_credit_grant g
             LEFT JOIN restricted r ON r.grant_id = g.id
             WHERE g.account_id = $1 AND g.remaining > 0 AND g.effective_at <= $2
               AND (g.lapses_at IS NULL OR g.lapses_at > $2)
               AND g.subscription_id IS NULL AND g.application = 'INVOICE'
               AND r.grant_id IS NULL
             ORDER BY g.id",
        )
        .bind::<SqlUuid, _>(account_id.as_uuid())
        .bind::<Timestamptz, _>(now)
        .load(conn)
        .await
        .attach("Error while reading the shared credit of an account")
        .into_db_result()
    }

    /// Expired grants of one account still holding value their rollover may carry once the
    /// expiry sweep reaches them: credit ended with its subscription never rolls over.
    pub async fn list_unswept_rollovers_for_account(
        conn: &mut PgConn,
        account_id: WalletId,
        now: DateTime<Utc>,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        g::customer_credit_grant
            .filter(g::account_id.eq(account_id))
            .filter(g::remaining.gt(0))
            .filter(g::rollover.is_not_null())
            .filter(g::expires_at.le(now))
            .filter(g::ends_at.is_null().or(g::ends_at.gt(now)))
            .order(g::id.asc())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading unswept credit rollovers")
            .into_db_result()
    }

    /// Grants an invoice's finalization funded from one of `source_kinds`, unlocked.
    pub async fn list_by_invoice_sources(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        source_kinds: &[&str],
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        g::customer_credit_grant
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::invoice_id.eq(invoice_id))
            .filter(g::source_kind.eq_any(source_kinds.to_vec()))
            .order(g::id.asc())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading invoice credit grants")
            .into_db_result()
    }

    /// The grants `grant_ids` rolled over into.
    pub async fn list_rolled_into(
        conn: &mut PgConn,
        tenant_id: TenantId,
        grant_ids: &[CreditGrantId],
    ) -> DbResult<Vec<CreditGrantId>> {
        use crate::schema::customer_credit_grant::dsl as g;
        if grant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let sources = grant_ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>();
        g::customer_credit_grant
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::source_kind.eq("ROLLOVER"))
            .filter(g::source_id.eq_any(sources))
            .order(g::id.asc())
            .select(g::id)
            .load::<Uuid>(conn)
            .await
            .attach("Error while reading rolled over credit")
            .into_db_result()
            .map(|ids| ids.into_iter().map(CreditGrantId::from).collect())
    }

    /// Which of `grant_ids` already rolled over into a grant.
    pub async fn list_rolled_over(
        conn: &mut PgConn,
        tenant_id: TenantId,
        grant_ids: &[CreditGrantId],
    ) -> DbResult<Vec<CreditGrantId>> {
        use crate::schema::customer_credit_grant::dsl as g;
        if grant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let sources = grant_ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>();
        g::customer_credit_grant
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::source_kind.eq("ROLLOVER"))
            .filter(g::source_id.eq_any(sources))
            .select(g::source_id)
            .load::<Uuid>(conn)
            .await
            .attach("Error while reading rolled over credit")
            .into_db_result()
            .map(|ids| ids.into_iter().map(CreditGrantId::from).collect())
    }

    /// Whether any grant of the customer has credit left: without one, nothing can be drawn.
    pub async fn any_unspent_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> DbResult<bool> {
        use crate::schema::customer_credit_grant::dsl as g;
        diesel::select(diesel::dsl::exists(
            g::customer_credit_grant
                .filter(g::tenant_id.eq(tenant_id))
                .filter(g::customer_id.eq(customer_id))
                .filter(g::remaining.gt(0)),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking whether a customer has credit left")
        .into_db_result()
    }

    /// Whether this account ever received a grant: an unfunded wallet computes to exactly zero,
    /// so this is what tells it apart from one drawn down to zero.
    pub async fn exists_for_account(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        account_id: WalletId,
    ) -> DbResult<bool> {
        use crate::schema::customer_credit_grant::dsl as g;
        diesel::select(diesel::dsl::exists(
            g::customer_credit_grant
                .filter(g::tenant_id.eq(tenant_id))
                .filter(g::customer_id.eq(customer_id))
                .filter(g::account_id.eq(account_id)),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking whether a credit account was ever funded")
        .into_db_result()
    }

    pub async fn list_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_grant::dsl as g;
        g::customer_credit_grant
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::customer_id.eq(customer_id))
            .order((g::created_at.asc(), g::id.asc()))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing customer credit grants")
            .into_db_result()
    }

    /// A page of a customer's grants, newest first, with the currency of their account. `state`
    /// is evaluated at `now`.
    #[allow(clippy::too_many_arguments)]
    pub async fn page_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: Option<&str>,
        subscription: Option<GrantSubscriptionFilter>,
        state: Option<CustomerCreditGrantStateFilter>,
        now: DateTime<Utc>,
        limit: i64,
        offset: i64,
    ) -> DbResult<Vec<(Self, String)>> {
        use crate::schema::customer_credit_account::dsl as a;
        use crate::schema::customer_credit_grant::dsl as g;
        grant_page_query!(tenant_id, customer_id, currency, subscription, state, now)
            .order((g::created_at.desc(), g::id.desc()))
            .limit(limit)
            .offset(offset)
            .select((Self::as_select(), a::currency))
            .load(conn)
            .await
            .attach("Error while listing customer credit grants")
            .into_db_result()
    }

    pub async fn count_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        currency: Option<&str>,
        subscription: Option<GrantSubscriptionFilter>,
        state: Option<CustomerCreditGrantStateFilter>,
        now: DateTime<Utc>,
    ) -> DbResult<i64> {
        use crate::schema::customer_credit_account::dsl as a;
        use crate::schema::customer_credit_grant::dsl as g;
        grant_page_query!(tenant_id, customer_id, currency, subscription, state, now)
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting customer credit grants")
            .into_db_result()
    }

    /// Every grant of one customer and currency, spent or not, with its products, locked in id
    /// order: the same order as `lock_drawable`, so the two never deadlock.
    pub async fn lock_all(
        conn: &mut PgConn,
        lock: &CustomerLock,
        currency: &str,
    ) -> DbResult<Vec<ScopedGrantRow>> {
        use diesel::sql_types::Uuid as SqlUuid;
        lock.on(conn);
        let (tenant_id, customer_id) = (lock.tenant_id(), lock.customer_id());
        let Some(account) =
            CustomerCreditAccountRow::find_by_currency(conn, tenant_id, customer_id, currency)
                .await?
        else {
            return Ok(Vec::new());
        };
        diesel::sql_query(scoped_grants_query(
            "c.tenant_id = $1 AND c.customer_id = $2 AND c.account_id = $3",
            "g.tenant_id = $1 AND g.customer_id = $2 AND g.account_id = $3",
            "FOR NO KEY UPDATE OF g",
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<SqlUuid, _>(customer_id.as_uuid())
        .bind::<SqlUuid, _>(account.id.as_uuid())
        .load(conn)
        .await
        .attach("Error while locking customer credit")
        .into_db_result()
    }

    /// Customers and currencies holding credit that lapsed (expired or ended) at or before
    /// `expired_by` and still has value, in `(tenant, customer, currency)` order after `after`,
    /// with the earliest such lapse and the customer's invoicing entity grace. Nothing is locked.
    pub async fn list_due_groups(
        conn: &mut PgConn,
        expired_by: DateTime<Utc>,
        after: Option<(TenantId, CustomerId, String)>,
        limit: i64,
    ) -> DbResult<Vec<DueExpiryCandidate>> {
        let (tenant, customer, currency) = after
            .map(|(t, c, currency)| (t.as_uuid(), c.as_uuid(), currency))
            .unwrap_or((Uuid::nil(), Uuid::nil(), String::new()));
        diesel::sql_query(
            "SELECT g.tenant_id, g.customer_id, a.currency, MIN(g.lapses_at) AS first_lapse,
                    MAX(ie.grace_period_hours) AS grace_period_hours,
                    bool_or(ie.consolidate_recurring_invoices) AS consolidate_recurring_invoices
             FROM customer_credit_grant g
             JOIN customer_credit_account a ON a.id = g.account_id
             JOIN customer c ON c.id = g.customer_id
             JOIN invoicing_entity ie ON ie.id = c.invoicing_entity_id
             WHERE g.remaining > 0 AND g.lapses_at <= $1
               AND (g.tenant_id, g.customer_id, a.currency) > ($2, $3, $4)
             GROUP BY g.tenant_id, g.customer_id, a.currency
             ORDER BY g.tenant_id, g.customer_id, a.currency
             LIMIT $5",
        )
        .bind::<sql_types::Timestamptz, _>(expired_by)
        .bind::<sql_types::Uuid, _>(tenant)
        .bind::<sql_types::Uuid, _>(customer)
        .bind::<sql_types::Text, _>(currency)
        .bind::<sql_types::BigInt, _>(limit)
        .get_results::<DueExpiryCandidate>(conn)
        .await
        .attach("Error while listing expired customer credit")
        .into_db_result()
    }

    /// Caller holds the grant lock and has checked the bounds; the table constraints refuse the rest.
    pub async fn move_remaining(
        conn: &mut PgConn,
        id: CreditGrantId,
        delta: i64,
    ) -> DbResult<Self> {
        use crate::schema::customer_credit_grant::dsl as g;
        diesel::update(g::customer_credit_grant.filter(g::id.eq(id)))
            .set(g::remaining.eq(g::remaining + delta))
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while moving customer credit")
            .into_db_result()
    }
}

/// `customer_credit_grant.end_reason` of credit that ended with its subscription.
pub const SUBSCRIPTION_ENDED: &str = "SUBSCRIPTION_ENDED";
/// `customer_credit_grant.end_reason` of plan credit an immediate plan change removed.
pub const PLAN_CHANGE_ENDED: &str = "PLAN_CHANGE";

impl CustomerCreditEntryRow {
    /// None when this movement was already recorded.
    pub async fn insert_new(
        conn: &mut PgConn,
        row: &CustomerCreditEntryRowNew,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_entry::dsl as e;
        diesel::insert_into(e::customer_credit_entry)
            .values(row)
            .on_conflict((e::grant_id, e::kind, e::source_id))
            .do_nothing()
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while recording a customer credit movement")
            .into_db_result()
    }

    /// What moved credit for an invoice: its own movements and the draws of the usage settlements
    /// it took over, oldest first.
    pub async fn list_by_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_entry::dsl as e;
        let mut entries: Vec<Self> = e::customer_credit_entry
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::invoice_id.eq(invoice_id))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading invoice credit movements")
            .into_db_result()?;
        let settlements: Vec<Uuid> =
            CreditUsageSettlementRow::list_by_invoice(conn, tenant_id, invoice_id)
                .await?
                .into_iter()
                .map(|settlement| settlement.id.as_uuid())
                .collect();
        if !settlements.is_empty() {
            entries.extend(
                e::customer_credit_entry
                    .filter(e::tenant_id.eq(tenant_id))
                    .filter(e::source_kind.eq("USAGE_SETTLEMENT"))
                    .filter(e::invoice_id.is_null())
                    .filter(e::source_id.eq_any(&settlements))
                    .select(Self::as_select())
                    .load(conn)
                    .await
                    .attach("Error while reading usage settlement credit movements")
                    .into_db_result()?,
            );
        }
        entries.sort_by_key(|entry| (entry.created_at, entry.id.as_uuid()));
        Ok(entries)
    }

    /// `list_by_invoice` restricted to the grants of the customer's wallets in `currency` (an ISO
    /// code or a credit unit code): an invoice can hold credit of several denominations.
    pub async fn list_by_invoice_in_currency(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        currency: &str,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_account::dsl as a;
        use crate::schema::customer_credit_grant::dsl as g;
        let entries = Self::list_by_invoice(conn, tenant_id, invoice_id).await?;
        if entries.is_empty() {
            return Ok(entries);
        }
        let mut grant_ids: Vec<CreditGrantId> = entries.iter().map(|e| e.grant_id).collect();
        grant_ids.sort_by_key(|id| id.as_uuid());
        grant_ids.dedup();
        let in_currency: Vec<CreditGrantId> = g::customer_credit_grant
            .inner_join(a::customer_credit_account.on(a::id.eq(g::account_id)))
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::id.eq_any(&grant_ids))
            .filter(a::currency.eq(currency))
            .select(g::id)
            .load(conn)
            .await
            .attach("Error while reading the denomination of invoice credit movements")
            .into_db_result()?;
        Ok(entries
            .into_iter()
            .filter(|entry| in_currency.contains(&entry.grant_id))
            .collect())
    }

    /// The denominations (ISO or credit unit codes) of the wallets `list_by_invoice` moved, sorted.
    pub async fn list_denominations_by_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<String>> {
        use crate::schema::customer_credit_account::dsl as a;
        use crate::schema::customer_credit_grant::dsl as g;
        let mut grant_ids: Vec<CreditGrantId> = Self::list_by_invoice(conn, tenant_id, invoice_id)
            .await?
            .into_iter()
            .map(|e| e.grant_id)
            .collect();
        if grant_ids.is_empty() {
            return Ok(Vec::new());
        }
        grant_ids.sort_by_key(|id| id.as_uuid());
        grant_ids.dedup();
        g::customer_credit_grant
            .inner_join(a::customer_credit_account.on(a::id.eq(g::account_id)))
            .filter(g::tenant_id.eq(tenant_id))
            .filter(g::id.eq_any(&grant_ids))
            .select(a::currency)
            .distinct()
            .order(a::currency.asc())
            .load(conn)
            .await
            .attach("Error while reading the denominations of invoice credit movements")
            .into_db_result()
    }

    /// The draws that paid usage settlements.
    pub async fn list_settlement_draws(
        conn: &mut PgConn,
        tenant_id: TenantId,
        settlement_ids: &[Uuid],
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_entry::dsl as e;
        e::customer_credit_entry
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::source_kind.eq("USAGE_SETTLEMENT"))
            .filter(e::kind.eq("DRAW"))
            .filter(e::source_id.eq_any(settlement_ids))
            .order(e::id.asc())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading usage settlement draws")
            .into_db_result()
    }

    pub async fn find_by_source(
        conn: &mut PgConn,
        grant_id: CreditGrantId,
        kind: &str,
        source_id: Uuid,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_entry::dsl as e;
        e::customer_credit_entry
            .filter(e::grant_id.eq(grant_id))
            .filter(e::kind.eq(kind))
            .filter(e::source_id.eq(source_id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a customer credit movement")
            .into_db_result()
    }

    /// Movements of one kind across a customer's grants, as `(grant, amount)` pairs.
    pub async fn list_amounts_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        kind: &str,
    ) -> DbResult<Vec<(CreditGrantId, i64)>> {
        use crate::schema::customer_credit_entry::dsl as e;
        e::customer_credit_entry
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::customer_id.eq(customer_id))
            .filter(e::kind.eq(kind))
            .select((e::grant_id, e::amount))
            .load(conn)
            .await
            .attach("Error while reading customer credit expiries")
            .into_db_result()
    }

    /// The most recent movement of each grant that has one, so a caller can tell whether a live
    /// estimate predates a ledger change it has not caught up with yet.
    pub async fn last_recorded_at_by_grant(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> DbResult<Vec<(CreditGrantId, DateTime<Utc>)>> {
        use crate::schema::customer_credit_entry::dsl as e;
        let rows: Vec<(CreditGrantId, Option<DateTime<Utc>>)> = e::customer_credit_entry
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::customer_id.eq(customer_id))
            .group_by(e::grant_id)
            .select((e::grant_id, diesel::dsl::max(e::created_at)))
            .load(conn)
            .await
            .attach("Error while reading the last credit ledger change per grant")
            .into_db_result()?;
        Ok(rows
            .into_iter()
            .filter_map(|(grant_id, at)| at.map(|at| (grant_id, at)))
            .collect())
    }

    /// When the latest movement of a grant happened, in business time.
    pub async fn last_occurred_at(
        conn: &mut PgConn,
        grant_id: CreditGrantId,
    ) -> DbResult<Option<DateTime<Utc>>> {
        use crate::schema::customer_credit_entry::dsl as e;
        e::customer_credit_entry
            .filter(e::grant_id.eq(grant_id))
            .select(diesel::dsl::max(e::occurred_at))
            .get_result(conn)
            .await
            .attach("Error while reading the last movement of a credit grant")
            .into_db_result()
    }

    pub async fn list_by_grant(
        conn: &mut PgConn,
        tenant_id: TenantId,
        grant_id: CreditGrantId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_entry::dsl as e;
        e::customer_credit_entry
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::grant_id.eq(grant_id))
            .order((e::created_at.asc(), e::id.asc()))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading credit grant movements")
            .into_db_result()
    }

    pub async fn list_by_note(
        conn: &mut PgConn,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_credit_entry::dsl as e;
        e::customer_credit_entry
            .filter(e::tenant_id.eq(tenant_id))
            .filter(e::credit_note_id.eq(credit_note_id))
            .order((e::created_at.asc(), e::id.asc()))
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading credit note credit movements")
            .into_db_result()
    }
}

impl CreditLineAllocationRowNew {
    /// Records how the credit an invoice drew (`credit_note_id` None), or a note gave back, sits
    /// on its lines; a replay of a document already recorded writes nothing.
    pub async fn record(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        credit_note_id: Option<CreditNoteId>,
        rows: &[CreditLineAllocationRowNew],
    ) -> DbResult<()> {
        use crate::schema::customer_credit_line_allocation::dsl as a;
        if rows.is_empty() {
            return Ok(());
        }
        let mut existing = a::customer_credit_line_allocation
            .filter(a::tenant_id.eq(tenant_id))
            .filter(a::invoice_id.eq(invoice_id))
            .into_boxed();
        existing = match credit_note_id {
            Some(note) => existing.filter(a::credit_note_id.eq(note)),
            None => existing.filter(a::credit_note_id.is_null()),
        };
        let recorded: i64 = existing
            .count()
            .get_result(conn)
            .await
            .attach("Error while reading a credit line allocation")
            .into_db_result()?;
        if recorded > 0 {
            return Ok(());
        }
        diesel::insert_into(a::customer_credit_line_allocation)
            .values(rows)
            .execute(conn)
            .await
            .attach("Error while recording a credit line allocation")
            .into_db_result()?;
        Ok(())
    }
}

impl CreditPurchaseRow {
    pub async fn insert(conn: &mut PgConn, row: &CreditPurchaseRowNew) -> DbResult<Self> {
        use crate::schema::customer_credit_purchase::dsl as p;
        diesel::insert_into(p::customer_credit_purchase)
            .values(row)
            .returning(Self::as_returning())
            .get_result(conn)
            .await
            .attach("Error while recording a credit purchase")
            .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: Uuid,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_purchase::dsl as p;
        p::customer_credit_purchase
            .filter(p::tenant_id.eq(tenant_id))
            .filter(p::id.eq(id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a credit purchase")
            .into_db_result()
    }

    /// The purchase an invoice sells; at most one per invoice.
    pub async fn find_by_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_purchase::dsl as p;
        p::customer_credit_purchase
            .filter(p::tenant_id.eq(tenant_id))
            .filter(p::invoice_id.eq(invoice_id))
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a credit purchase")
            .into_db_result()
    }
}

impl HistoricalBalanceTxRow {
    /// Whether main added this note's credited amount to the retired scalar balance.
    pub async fn note_credited_retired_balance(
        conn: &mut PgConn,
        tenant_id: TenantId,
        credit_note_id: CreditNoteId,
    ) -> DbResult<bool> {
        use crate::schema::credit_note::dsl as cn;
        cn::credit_note
            .filter(cn::tenant_id.eq(tenant_id))
            .filter(cn::id.eq(credit_note_id))
            .select(cn::credited_retired_balance)
            .first(conn)
            .await
            .optional()
            .map(|flag| flag.unwrap_or(false))
            .attach("Error while reading whether a credit note credited the retired balance")
            .into_db_result()
    }

    pub async fn deductions_for_invoice(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        invoice_id: InvoiceId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::customer_balance_tx::dsl as t;
        t::customer_balance_tx
            .filter(t::tenant_id.eq(tenant_id))
            .filter(t::customer_id.eq(customer_id))
            .filter(t::invoice_id.eq(invoice_id))
            .filter(t::amount_cents.lt(0))
            .order(t::id.asc())
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading historical balance deductions")
            .into_db_result()
    }
}

/// Entries of one account plus the unspent value of expired grants the sweep has not recorded yet,
/// less what it will roll over (`$3` grants, `$4` carried amounts); `$5` is the reading's instant.
/// An unrecorded expiry is dated as the sweep will date it (at the expiry, or at the grant's last
/// movement when value came back after it) and follows what was recorded at the same instant.
pub(crate) const STATEMENT_MOVEMENTS: &str = "
    SELECT e.id, g.account_id, e.grant_id, e.kind, e.amount, coalesce(e.invoice_id, s.invoice_id) AS invoice_id,
           e.credit_note_id,
           g.reason AS grant_reason, e.reason AS entry_reason, g.reference AS grant_reference,
           coalesce(e.created_by, CASE WHEN e.kind = 'FUND' THEN g.created_by END) AS created_by,
           e.occurred_at, e.created_at AS recorded_at,
           e.cost_minor, g.cost_currency, g.application AS grant_application,
           g.subscription_id AS grant_subscription_id, 0 AS unrecorded
    FROM customer_credit_entry e
    JOIN customer_credit_grant g ON g.id = e.grant_id
    LEFT JOIN credit_usage_settlement s
           ON e.source_kind = 'USAGE_SETTLEMENT' AND s.id = e.source_id AND s.tenant_id = e.tenant_id
    WHERE e.tenant_id = $1 AND g.tenant_id = $1 AND g.account_id = ANY($2)
      AND e.customer_id IN (SELECT a.customer_id FROM customer_credit_account a
                            WHERE a.tenant_id = $1 AND a.id = ANY($2))
    UNION ALL
    SELECT g.id, g.account_id, g.id, 'EXPIRY', -(g.remaining - coalesce(r.carried, 0)), NULL, NULL,
           g.reason, NULL, g.reference, NULL,
           greatest(g.lapses_at, l.last_at), greatest(g.lapses_at, l.last_at),
           -- The cost the lapsing remainder still holds, as the sweep's EXPIRY entry will record it.
           (div(2 * (g.amount - g.remaining + coalesce(r.carried, 0))::numeric * g.cost_minor + g.amount,
                2 * g.amount::numeric)
               - g.cost_minor)::bigint,
           g.cost_currency, g.application, g.subscription_id, 1
    FROM customer_credit_grant g
    LEFT JOIN unnest($3::uuid[], $4::bigint[]) AS r(grant_id, carried) ON r.grant_id = g.id
    LEFT JOIN (SELECT le.grant_id, max(le.occurred_at) AS last_at
               FROM customer_credit_entry le
               JOIN customer_credit_grant lg ON lg.id = le.grant_id
               WHERE le.tenant_id = $1 AND lg.tenant_id = $1 AND lg.account_id = ANY($2)
                 AND lg.remaining > 0 AND lg.lapses_at <= $5
               GROUP BY le.grant_id) l ON l.grant_id = g.id
    WHERE g.tenant_id = $1 AND g.account_id = ANY($2) AND g.remaining > coalesce(r.carried, 0)
      AND g.lapses_at <= $5";

fn account_uuids(account_ids: &[WalletId]) -> Vec<Uuid> {
    account_ids.iter().map(|id| id.as_uuid()).collect()
}

fn rolling_arrays(rolling: &[(CreditGrantId, i64)]) -> (Vec<Uuid>, Vec<i64>) {
    rolling
        .iter()
        .map(|(grant_id, carried)| (grant_id.as_uuid(), *carried))
        .unzip()
}

impl CustomerCreditStatementRow {
    /// One statement over the given accounts as of `now`, newest first; `sequence` and
    /// `balance_after` follow their combined business-time order, then the recording order.
    /// `rolling` holds, per expired grant the sweep has not rolled over yet, what it will carry.
    pub async fn list_for_accounts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        account_ids: &[WalletId],
        rolling: &[(CreditGrantId, i64)],
        now: DateTime<Utc>,
        limit: i64,
        offset: i64,
    ) -> DbResult<Vec<Self>> {
        use diesel::sql_types::{Array, BigInt, Timestamptz, Uuid as SqlUuid};
        let (rolling_ids, carried) = rolling_arrays(rolling);
        diesel::sql_query(format!(
            "SELECT m.*,
                    row_number() OVER w AS sequence,
                    (sum(m.amount) OVER w)::bigint AS balance_after
             FROM ({STATEMENT_MOVEMENTS}) m
             WINDOW w AS (ORDER BY m.occurred_at, m.unrecorded, m.recorded_at, m.id
                           ROWS UNBOUNDED PRECEDING)
             ORDER BY sequence DESC
             LIMIT $6 OFFSET $7"
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(account_uuids(account_ids))
        .bind::<Array<SqlUuid>, _>(rolling_ids)
        .bind::<Array<BigInt>, _>(carried)
        .bind::<Timestamptz, _>(now)
        .bind::<BigInt, _>(limit)
        .bind::<BigInt, _>(offset)
        .load(conn)
        .await
        .attach("Error while reading a customer credit statement")
        .into_db_result()
    }

    pub async fn count_for_accounts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        account_ids: &[WalletId],
        rolling: &[(CreditGrantId, i64)],
        now: DateTime<Utc>,
    ) -> DbResult<i64> {
        use diesel::sql_types::{Array, BigInt, Timestamptz, Uuid as SqlUuid};
        let (rolling_ids, carried) = rolling_arrays(rolling);
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            total: i64,
        }
        diesel::sql_query(format!(
            "SELECT count(*) AS total FROM ({STATEMENT_MOVEMENTS}) m"
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(account_uuids(account_ids))
        .bind::<Array<SqlUuid>, _>(rolling_ids)
        .bind::<Array<BigInt>, _>(carried)
        .bind::<Timestamptz, _>(now)
        .get_result::<Count>(conn)
        .await
        .map(|row| row.total)
        .attach("Error while counting a customer credit statement")
        .into_db_result()
    }
}

/// Per active subscription of `customers` (a subquery), each denomination it charges usage in,
/// upper-cased, with what its plan version does once the credit runs out. A component without a
/// price (a legacy one) counts in the subscription currency.
fn plan_credit_rules(customers: &str) -> String {
    format!(
        "SELECT DISTINCT s.customer_id, s.id AS subscription_id, p.name AS plan_name,
                upper(btrim(COALESCE(cu.code, s.currency))) AS denomination,
                COALESCE(pv.credit_exhausted_rule, 'INVOICE') AS credit_exhausted_rule,
                pv.credit_exhausted_caps
         FROM subscription s
         JOIN plan_version pv ON pv.id = s.plan_version_id AND pv.tenant_id = s.tenant_id
         JOIN plan p ON p.id = pv.plan_id
         JOIN subscription_component sc ON sc.subscription_id = s.id
         LEFT JOIN price pr ON pr.id = sc.price_id
         LEFT JOIN credit_unit cu ON cu.id = pr.credit_unit_id
         WHERE s.tenant_id = $1 AND s.customer_id IN ({customers})
           AND s.status IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE')
           AND (pr.id IS NULL OR pr.pricing->>'type' IN ('Usage', 'Capacity'))"
    )
}

impl PlanCreditRuleRow {
    /// The plan rules of every active subscription of the customers, by denomination charged.
    pub async fn list_for_customers(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_ids: &[CustomerId],
    ) -> DbResult<Vec<Self>> {
        use diesel::sql_types::{Array, Uuid as SqlUuid};
        if customer_ids.is_empty() {
            return Ok(Vec::new());
        }
        diesel::sql_query(format!(
            "{} ORDER BY plan_name, subscription_id, denomination",
            plan_credit_rules("SELECT unnest($2::uuid[])")
        ))
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Array<SqlUuid>, _>(
            customer_ids
                .iter()
                .map(|id| id.as_uuid())
                .collect::<Vec<_>>(),
        )
        .load(conn)
        .await
        .attach("Error while listing the plan credit rules of customers")
        .into_db_result()
    }
}

const ROSTER_SCOPE: &str = "
    SELECT a.id, a.customer_id, c.name AS customer_name, c.alias AS customer_alias, a.currency,
           a.credit_unit_id, a.low_balance_thresholds_minor, a.created_at
    FROM customer_credit_account a
    JOIN customer c ON c.id = a.customer_id AND c.tenant_id = a.tenant_id
    WHERE a.tenant_id = $1
      AND ($2::text IS NULL OR c.name ILIKE $2 OR c.alias ILIKE $2)";

const ROSTER_ORDER: &str = "ORDER BY customer_name, currency, id";

const ROSTER_PAUSES_AT_ZERO: &str = "(a.usage_at_zero = 'ALWAYS_PAUSE'
                        OR (a.usage_at_zero = 'FOLLOW_PLAN' AND EXISTS (
                            SELECT 1 FROM plan_rules pr
                            WHERE pr.customer_id = a.customer_id
                              AND pr.denomination = upper(btrim(a.currency))
                              AND (pr.credit_exhausted_rule = 'PAUSE'
                                OR (pr.credit_exhausted_rule = 'INVOICE_UP_TO'
                                    AND NOT pr.credit_exhausted_caps ? pr.denomination)))))";

/// Each wallet of `scope` classified from every wallet its customer holds in the same currency,
/// grouped once per CTE (no per-row subquery). The status mirrors the store's `WalletStatus::classify`.
/// Without the usage controls (`live` false) the projection and automatic top-ups they wrote are
/// ignored and no wallet pauses usage: the ledger alone classifies, as the store's own hook does.
fn roster_query(
    scope_tail: &str,
    status: &str,
    auto_top_up_failing: &str,
    tail: &str,
    live: bool,
) -> String {
    let plan_rules = plan_credit_rules("SELECT customer_id FROM scope");
    let (pauses_at_zero, projected_on, top_up_on) = if live {
        (
            ROSTER_PAUSES_AT_ZERO,
            "pr.account_id = r.id",
            "t.account_id = s.id",
        )
    } else {
        ("false", "false", "false")
    };
    format!(
        "WITH scope AS ({ROSTER_SCOPE} {scope_tail}),
         plan_rules AS ({plan_rules}),
         related AS (
             SELECT a.id, a.customer_id, upper(btrim(a.currency)) AS currency_key,
                    a.usage_at_zero,
                    {pauses_at_zero} AS pauses_at_zero
             FROM customer_credit_account a
             WHERE a.tenant_id = $1 AND a.customer_id IN (SELECT customer_id FROM scope)),
         balances AS (
             SELECT g.account_id,
                    COALESCE(sum(g.remaining) FILTER (WHERE g.effective_at <= $3), 0)::bigint
                        AS available_minor,
                    COALESCE(sum(g.remaining) FILTER (WHERE g.effective_at > $3), 0)::bigint
                        AS pending_minor
             FROM customer_credit_grant g
             JOIN related r ON r.id = g.account_id
             WHERE g.tenant_id = $1 AND (g.lapses_at IS NULL OR g.lapses_at > $3)
             GROUP BY g.account_id),
         measured AS (
             SELECT r.id, r.customer_id, r.currency_key, r.usage_at_zero, r.pauses_at_zero,
                    COALESCE(b.available_minor, 0)::bigint AS available_minor,
                    COALESCE(b.pending_minor, 0)::bigint AS pending_minor,
                    pr.account_id IS NOT NULL AS projected,
                    pr.live_available_minor, pr.pending_minor AS projected_usage_minor,
                    pr.overage_minor, pr.computed_at
             FROM related r
             LEFT JOIN balances b ON b.account_id = r.id
             LEFT JOIN credit_balance_projection pr ON {projected_on}),
         currencies AS (
             SELECT customer_id, currency_key,
                    bool_and(projected) AS projected,
                    bool_or(pauses_at_zero) AS blocks,
                    (CASE WHEN bool_and(projected) THEN sum(live_available_minor)
                          ELSE sum(available_minor) END)::bigint AS balance_minor,
                    (CASE WHEN bool_and(projected) THEN max(overage_minor) END)::bigint
                        AS overage_minor
             FROM measured
             GROUP BY customer_id, currency_key),
         weighed AS (
             SELECT m.*, cu.projected AS currency_projected, cu.blocks,
                    cu.balance_minor AS currency_balance_minor,
                    cu.overage_minor AS currency_overage_minor,
                    CASE WHEN cu.projected THEN m.live_available_minor ELSE m.available_minor END
                        AS balance_minor
             FROM measured m
             JOIN currencies cu
                 ON cu.customer_id = m.customer_id AND cu.currency_key = m.currency_key),
         classified AS (
             SELECT s.*, w.available_minor, w.pending_minor, w.usage_at_zero, w.pauses_at_zero,
                    CASE WHEN w.currency_projected THEN w.live_available_minor END
                        AS live_balance_minor,
                    CASE WHEN w.currency_projected THEN w.projected_usage_minor END
                        AS pending_usage_minor,
                    w.currency_overage_minor AS uncovered_usage_minor,
                    CASE WHEN w.currency_projected THEN w.computed_at END AS live_as_of,
                    (t.account_id IS NOT NULL AND t.parked_at IS NULL) AS auto_top_up_enabled,
                    COALESCE(t.parked_at IS NULL AND t.suspended_at IS NOT NULL, false)
                        AS auto_top_up_suspended,
                    COALESCE(t.parked_at IS NULL AND t.suspended_at IS NULL
                        AND t.consecutive_failures > 0, false) AS auto_top_up_failing,
                    CASE WHEN t.parked_at IS NULL THEN t.threshold_minor END
                        AS auto_top_up_threshold_minor,
                    CASE
                        WHEN w.blocks AND w.currency_balance_minor <= 0 THEN 'USAGE_BLOCKED'
                        WHEN w.balance_minor <= 0 THEN 'DEPLETED'
                        -- thresholds are stored ascending: the last is the highest
                        WHEN w.balance_minor
                                < s.low_balance_thresholds_minor[cardinality(s.low_balance_thresholds_minor)]
                            OR (t.parked_at IS NULL AND w.balance_minor <= t.threshold_minor)
                            THEN 'LOW'
                        ELSE 'HEALTHY'
                    END AS status
             FROM scope s
             JOIN weighed w ON w.id = s.id
             LEFT JOIN customer_credit_auto_top_up t ON {top_up_on})
         SELECT * FROM classified
         WHERE ({status}::text IS NULL OR status = {status}::text)
           AND (NOT {auto_top_up_failing}::bool OR auto_top_up_failing OR auto_top_up_suspended)
         {tail}"
    )
}

/// The listed page, each wallet flagged stale when its ledger changed after its live estimate
/// was computed (the rule of the store's `wallet_health`). Measured on the page only.
pub fn roster_page_query(
    scope_tail: &str,
    status: &str,
    auto_top_up_failing: &str,
    tail: &str,
    live: bool,
) -> String {
    let page = roster_query(scope_tail, status, auto_top_up_failing, tail, live);
    format!(
        "WITH page AS ({page}),
         changed AS (
             SELECT g.account_id, max(e.created_at) AS last_change_at
             FROM customer_credit_entry e
             JOIN customer_credit_grant g ON g.id = e.grant_id
             WHERE e.tenant_id = $1 AND e.customer_id IN (SELECT customer_id FROM page)
               AND g.account_id IN (SELECT id FROM page)
             GROUP BY g.account_id)
         SELECT p.*, COALESCE(ch.last_change_at > p.live_as_of, false) AS live_stale
         FROM page p
         LEFT JOIN changed ch ON ch.account_id = p.id
         {ROSTER_ORDER}"
    )
}

impl CustomerCreditRosterRow {
    /// `status` and `auto_top_up_failing` filter the rows (failing includes stopped top-ups).
    /// `live`: the edition runs the usage controls.
    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        search: Option<&str>,
        status: Option<&str>,
        auto_top_up_failing: bool,
        now: DateTime<Utc>,
        limit: i64,
        offset: i64,
        live: bool,
    ) -> DbResult<Vec<Self>> {
        use diesel::sql_types::{BigInt, Bool, Nullable, Text, Timestamptz, Uuid as SqlUuid};
        let page = format!("{ROSTER_ORDER} LIMIT $4 OFFSET $5");
        // Unfiltered, the page is cut before anything is measured; a filter needs every wallet.
        let query = if status.is_none() && !auto_top_up_failing {
            roster_page_query(
                "ORDER BY c.name, a.currency, a.id LIMIT $4 OFFSET $5",
                "$6",
                "$7",
                "",
                live,
            )
        } else {
            roster_page_query("", "$6", "$7", &page, live)
        };
        diesel::sql_query(query)
            .bind::<SqlUuid, _>(tenant_id.as_uuid())
            .bind::<Nullable<Text>, _>(search.map(|term| format!("%{term}%")))
            .bind::<Timestamptz, _>(now)
            .bind::<BigInt, _>(limit)
            .bind::<BigInt, _>(offset)
            .bind::<Nullable<Text>, _>(status)
            .bind::<Bool, _>(auto_top_up_failing)
            .load(conn)
            .await
            .attach("Error while listing customer credit wallets")
            .into_db_result()
    }

    pub async fn count(
        conn: &mut PgConn,
        tenant_id: TenantId,
        search: Option<&str>,
        status: Option<&str>,
        auto_top_up_failing: bool,
        now: DateTime<Utc>,
        live: bool,
    ) -> DbResult<i64> {
        use diesel::sql_types::{BigInt, Bool, Nullable, Text, Timestamptz, Uuid as SqlUuid};
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            total: i64,
        }
        let search = search.map(|term| format!("%{term}%"));
        let counted = if status.is_none() && !auto_top_up_failing {
            diesel::sql_query(format!("SELECT count(*) AS total FROM ({ROSTER_SCOPE}) p"))
                .bind::<SqlUuid, _>(tenant_id.as_uuid())
                .bind::<Nullable<Text>, _>(search)
                .get_result::<Count>(conn)
                .await
        } else {
            diesel::sql_query(format!(
                "SELECT count(*) AS total FROM ({}) p",
                roster_query("", "$4", "$5", "", live)
            ))
            .bind::<SqlUuid, _>(tenant_id.as_uuid())
            .bind::<Nullable<Text>, _>(search)
            .bind::<Timestamptz, _>(now)
            .bind::<Nullable<Text>, _>(status)
            .bind::<Bool, _>(auto_top_up_failing)
            .get_result::<Count>(conn)
            .await
        };
        counted
            .map(|row| row.total)
            .attach("Error while counting customer credit wallets")
            .into_db_result()
    }
}

impl CreditUnrecoveredRow {
    /// What a wallet's loss on one money flow was last reported at, without locking it.
    pub async fn reported(
        conn: &mut PgConn,
        tenant_id: TenantId,
        account_id: WalletId,
        source_id: Uuid,
    ) -> DbResult<i64> {
        use crate::schema::customer_credit_unrecovered::dsl as u;
        u::customer_credit_unrecovered
            .filter(u::tenant_id.eq(tenant_id))
            .filter(u::account_id.eq(account_id))
            .filter(u::source_id.eq(source_id))
            .select(u::reported)
            .first(conn)
            .await
            .optional()
            .map(Option::unwrap_or_default)
            .attach("Error while reading a reported credit loss")
            .into_db_result()
    }

    pub async fn find_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        account_id: WalletId,
        source_id: Uuid,
    ) -> DbResult<Option<Self>> {
        use crate::schema::customer_credit_unrecovered::dsl as u;
        {
            use crate::schema::customer_credit_account::dsl as a;
            let (tenant, customer): (TenantId, CustomerId) = a::customer_credit_account
                .filter(a::id.eq(account_id))
                .select((a::tenant_id, a::customer_id))
                .first(conn)
                .await
                .attach("Error while reading the customer of a credit wallet")
                .into_db_result()?;
            lock.covers(conn, tenant, customer)?;
        }
        let tenant_id = lock.tenant_id();
        u::customer_credit_unrecovered
            .filter(u::tenant_id.eq(tenant_id))
            .filter(u::account_id.eq(account_id))
            .filter(u::source_id.eq(source_id))
            .for_update()
            .select(Self::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while locking a reported credit loss")
            .into_db_result()
    }

    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::customer_credit_unrecovered::dsl as u;
        diesel::insert_into(u::customer_credit_unrecovered)
            .values(self)
            .on_conflict((u::tenant_id, u::account_id, u::source_id))
            .do_update()
            .set((
                u::reported.eq(self.reported),
                u::notices.eq(self.notices),
                u::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while recording a reported credit loss")
            .into_db_result()
    }
}
