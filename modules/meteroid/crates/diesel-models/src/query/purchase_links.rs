use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::purchase_links::{
    AbandonedPurchaseRow, ListedPurchaseLinkRow, PurchaseBindingRow, PurchaseLinkRow,
    PurchaseLinkRowNew, PurchaseLinkRowUpdate, PurchaseLinkSessionRestart, PurchaseLinkSessionRow,
    PurchaseLinkSessionRowNew, PurchaseLinkStatsRow, PurchasePowSpentRowNew,
};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{
    CheckoutSessionId, CustomerId, PlanId, PurchaseLinkId, PurchaseLinkSessionId, SubscriptionId,
    TenantId,
};
use diesel::{
    ExpressionMethods, OptionalExtension, QueryDsl, QueryableByName, SelectableHelper, sql_query,
    sql_types,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

/// Shared by the listing and the locked re-check of abandoned purchases (`pls` joined to its
/// checkout `cs`).
const ABANDONED: &str = r#"pls.cleaned_up_at IS NULL
              AND (
                cs.id IS NULL
                OR (
                  cs.subscription_id IS NULL
                  AND (
                    cs.status IN ('EXPIRED', 'CANCELLED')
                    OR (cs.status = 'CREATED' AND cs.expires_at < NOW())
                  )
                )
              )
              AND NOT EXISTS (SELECT 1 FROM subscription s WHERE s.customer_id = pls.customer_id)
              AND NOT EXISTS (SELECT 1 FROM invoice i WHERE i.customer_id = pls.customer_id)
              AND NOT EXISTS (
                SELECT 1 FROM payment_transaction pt
                JOIN checkout_session c ON c.id = pt.checkout_session_id
                WHERE c.customer_id = pls.customer_id
                  AND pt.status NOT IN ('FAILED', 'CANCELLED')
              )
              AND NOT EXISTS (
                SELECT 1 FROM checkout_session c
                WHERE c.customer_id = pls.customer_id
                  AND (
                    c.status = 'AWAITING_PAYMENT'
                    OR (c.status = 'CREATED' AND (c.expires_at IS NULL OR c.expires_at >= NOW()))
                  )
              )"#;

impl PurchaseLinkRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PurchaseLinkRow> {
        use crate::schema::purchase_link::dsl as pl;

        let query = diesel::insert_into(pl::purchase_link).values(self);
        crate::debug_sql(&query);

        query
            .returning(PurchaseLinkRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting purchase link")
            .into_db_result()
    }
}

impl PurchaseLinkRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> DbResult<PurchaseLinkRow> {
        use crate::schema::purchase_link::dsl as pl;

        pl::purchase_link
            .filter(pl::id.eq(id))
            .filter(pl::tenant_id.eq(tenant_id))
            .select(PurchaseLinkRow::as_select())
            .first(conn)
            .await
            .attach("Error while fetching purchase link")
            .into_db_result()
    }

    /// The token is globally unique, so this is the only unscoped lookup: it is how an
    /// anonymous buyer reaches a tenant.
    pub async fn find_unarchived_by_token(
        conn: &mut PgConn,
        token: &str,
    ) -> DbResult<Option<PurchaseLinkRow>> {
        use crate::schema::purchase_link::dsl as pl;

        pl::purchase_link
            .filter(pl::token.eq(token))
            .filter(pl::archived_at.is_null())
            .select(PurchaseLinkRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching purchase link by token")
            .into_db_result()
    }

    /// Links with the name of their plan, most recent first.
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_id: Option<PlanId>,
        include_archived: bool,
    ) -> DbResult<Vec<(PurchaseLinkRow, String)>> {
        use crate::schema::plan::dsl as p;
        use crate::schema::purchase_link::dsl as pl;

        let mut query = pl::purchase_link
            .inner_join(p::plan)
            .filter(pl::tenant_id.eq(tenant_id))
            .order_by(pl::created_at.desc())
            .select((PurchaseLinkRow::as_select(), p::name))
            .into_boxed();

        if let Some(plan_id) = plan_id {
            query = query.filter(pl::plan_id.eq(plan_id));
        }
        if !include_archived {
            query = query.filter(pl::archived_at.is_null());
        }

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing purchase links")
            .into_db_result()
    }

    /// `None` when the link doesn't exist or is archived (archived links are frozen).
    pub async fn update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PurchaseLinkId,
        update: PurchaseLinkRowUpdate,
    ) -> DbResult<Option<PurchaseLinkRow>> {
        use crate::schema::purchase_link::dsl as pl;

        diesel::update(pl::purchase_link)
            .filter(pl::id.eq(id))
            .filter(pl::tenant_id.eq(tenant_id))
            .filter(pl::archived_at.is_null())
            .set((
                pl::name.eq(update.name),
                pl::allow_coupon_codes.eq(update.allow_coupon_codes),
                pl::listed.eq(update.listed),
                pl::success_url.eq(update.success_url),
                pl::cancel_url.eq(update.cancel_url),
                pl::require_billing_address.eq(update.require_billing_address),
                pl::tax_id_collection.eq(update.tax_id_collection),
                pl::only_buyer_type.eq(update.only_buyer_type),
                pl::updated_at.eq(diesel::dsl::now),
            ))
            .returning(PurchaseLinkRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating purchase link")
            .into_db_result()
    }

    pub async fn set_token(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PurchaseLinkId,
        token: &str,
    ) -> DbResult<Option<PurchaseLinkRow>> {
        use crate::schema::purchase_link::dsl as pl;

        diesel::update(pl::purchase_link)
            .filter(pl::id.eq(id))
            .filter(pl::tenant_id.eq(tenant_id))
            .filter(pl::archived_at.is_null())
            .set((pl::token.eq(token), pl::updated_at.eq(diesel::dsl::now)))
            .returning(PurchaseLinkRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while regenerating purchase link token")
            .into_db_result()
    }

    /// Idempotent: an already archived link keeps its first archive date.
    pub async fn archive(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PurchaseLinkId,
    ) -> DbResult<PurchaseLinkRow> {
        use crate::schema::purchase_link::dsl as pl;

        diesel::update(pl::purchase_link)
            .filter(pl::id.eq(id))
            .filter(pl::tenant_id.eq(tenant_id))
            .filter(pl::archived_at.is_null())
            .set((
                pl::archived_at.eq(diesel::dsl::now),
                pl::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)
            .await
            .attach("Error while archiving purchase link")
            .into_db_result()?;

        Self::find_by_id(conn, tenant_id, id).await
    }

    /// Listed, unarchived links whose plan can currently be bought.
    pub async fn list_listed(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<ListedPurchaseLinkRow>> {
        use crate::enums::{PlanStatusEnum, PlanTypeEnum};
        use crate::schema::plan::dsl as p;
        use crate::schema::purchase_link::dsl as pl;

        let query = pl::purchase_link
            .inner_join(p::plan)
            .filter(pl::tenant_id.eq(tenant_id))
            .filter(pl::listed.eq(true))
            .filter(pl::archived_at.is_null())
            .filter(p::issues_license.eq(true))
            .filter(p::status.eq(PlanStatusEnum::Active))
            .filter(p::plan_type.ne(PlanTypeEnum::Custom))
            .filter(p::archived_at.is_null())
            .filter(p::active_version_id.is_not_null())
            .order_by(pl::created_at.asc())
            .select((pl::id, pl::name, p::name, p::description, pl::token));

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing listed purchase links")
            .into_db_result()
    }

    /// `Some(allowed)` when the checkout session was started from a purchase link.
    pub async fn coupon_codes_allowed_for_checkout(
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
    ) -> DbResult<Option<bool>> {
        use crate::schema::purchase_link::dsl as pl;
        use crate::schema::purchase_link_session::dsl as pls;

        pls::purchase_link_session
            .inner_join(pl::purchase_link)
            .filter(pls::tenant_id.eq(tenant_id))
            .filter(pls::checkout_session_id.eq(checkout_session_id))
            .select(pl::allow_coupon_codes)
            .first::<bool>(conn)
            .await
            .optional()
            .attach("Error while resolving the purchase link of a checkout session")
            .into_db_result()
    }
}

impl PurchasePowSpentRowNew {
    /// Fails with a unique violation when the challenge was already spent.
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::purchase_pow_spent::dsl as ps;

        diesel::insert_into(ps::purchase_pow_spent)
            .values(self)
            .execute(conn)
            .await
            .attach("Error while spending a proof of work")
            .into_db_result()
    }

    pub async fn delete_expired(conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::purchase_pow_spent::dsl as ps;

        diesel::delete(ps::purchase_pow_spent.filter(ps::expires_at.lt(diesel::dsl::now)))
            .execute(conn)
            .await
            .attach("Error while purging spent proofs of work")
            .into_db_result()
    }
}

impl PurchaseLinkSessionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PurchaseLinkSessionRow> {
        use crate::schema::purchase_link_session::dsl as pls;

        let query = diesel::insert_into(pls::purchase_link_session).values(self);
        crate::debug_sql(&query);

        query
            .returning(PurchaseLinkSessionRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting purchase link session")
            .into_db_result()
    }
}

impl PurchaseLinkSessionRow {
    pub async fn find_by_attempt(
        conn: &mut PgConn,
        tenant_id: TenantId,
        purchase_link_id: PurchaseLinkId,
        attempt_id: &str,
    ) -> DbResult<Option<PurchaseLinkSessionRow>> {
        use crate::schema::purchase_link_session::dsl as pls;

        pls::purchase_link_session
            .filter(pls::tenant_id.eq(tenant_id))
            .filter(pls::purchase_link_id.eq(purchase_link_id))
            .filter(pls::attempt_id.eq(attempt_id))
            .select(PurchaseLinkSessionRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching purchase attempt")
            .into_db_result()
    }

    /// Repoints an attempt at a new checkout, provided nobody moved it since it was read
    /// (`previous_checkout`) and, when its customer is reused, the cleanup didn't archive it.
    /// Returns the number of rows updated (0 = lost a race).
    pub async fn restart(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PurchaseLinkSessionId,
        previous_checkout: Option<CheckoutSessionId>,
        reuses_customer: bool,
        restart: PurchaseLinkSessionRestart,
    ) -> DbResult<usize> {
        sql_query(
            r#"
            UPDATE purchase_link_session
            SET customer_id = $3,
                checkout_session_id = $4,
                license_instance_id = $5,
                license_instance_public_key = $6,
                created_at = NOW(),
                cleaned_up_at = NULL
            WHERE id = $1 AND tenant_id = $2
              AND checkout_session_id IS NOT DISTINCT FROM $7
              AND (NOT $8 OR cleaned_up_at IS NULL)
            "#,
        )
        .bind::<sql_types::Uuid, _>(id)
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(restart.customer_id)
        .bind::<sql_types::Uuid, _>(restart.checkout_session_id)
        .bind::<sql_types::Nullable<sql_types::Text>, _>(restart.license_instance_id)
        .bind::<sql_types::Nullable<sql_types::Text>, _>(restart.license_instance_public_key)
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(previous_checkout)
        .bind::<sql_types::Bool, _>(reuses_customer)
        .execute(conn)
        .await
        .attach("Error while restarting purchase attempt")
        .into_db_result()
    }

    /// Declined checkout charges across every checkout of the customer.
    pub async fn count_declines_for_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> DbResult<i64> {
        #[derive(QueryableByName)]
        struct CountRow {
            #[diesel(sql_type = sql_types::BigInt)]
            count: i64,
        }

        sql_query(
            r#"
            SELECT COUNT(*) AS count
            FROM payment_transaction pt
            JOIN checkout_session c ON c.id = pt.checkout_session_id
            WHERE c.tenant_id = $1 AND c.customer_id = $2 AND pt.status = 'FAILED'
            "#,
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(customer_id)
        .get_result::<CountRow>(conn)
        .await
        .map(|r| r.count)
        .attach("Error while counting declined purchase payments")
        .into_db_result()
    }

    pub async fn stats(
        conn: &mut PgConn,
        tenant_id: TenantId,
        link_ids: &[PurchaseLinkId],
    ) -> DbResult<Vec<PurchaseLinkStatsRow>> {
        if link_ids.is_empty() {
            return Ok(vec![]);
        }
        sql_query(
            r#"
            SELECT pls.purchase_link_id,
                   COUNT(*) AS started,
                   COUNT(cs.completed_at) AS completed
            FROM purchase_link_session pls
            LEFT JOIN checkout_session cs ON cs.id = pls.checkout_session_id
            WHERE pls.tenant_id = $1 AND pls.purchase_link_id = ANY($2)
            GROUP BY pls.purchase_link_id
            "#,
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Array<sql_types::Uuid>, _>(link_ids)
        .get_results(conn)
        .await
        .attach("Error while computing purchase link stats")
        .into_db_result()
    }

    pub async fn find_binding_by_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> DbResult<Option<PurchaseBindingRow>> {
        sql_query(
            r#"
            SELECT cs.subscription_id,
                   pls.license_instance_id AS instance_id,
                   pls.license_instance_public_key AS instance_public_key
            FROM purchase_link_session pls
            JOIN checkout_session cs ON cs.id = pls.checkout_session_id
            WHERE pls.tenant_id = $1 AND cs.tenant_id = $1
              AND cs.subscription_id = $2
              AND pls.license_instance_id IS NOT NULL
              AND pls.license_instance_public_key IS NOT NULL
            ORDER BY pls.created_at DESC
            LIMIT 1
            "#,
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(subscription_id)
        .get_result(conn)
        .await
        .optional()
        .attach("Error while finding the purchase binding of a subscription")
        .into_db_result()
    }

    pub async fn find_binding_by_instance(
        conn: &mut PgConn,
        tenant_id: TenantId,
        instance_id: &str,
    ) -> DbResult<Vec<PurchaseBindingRow>> {
        sql_query(
            r#"
            SELECT cs.subscription_id,
                   pls.license_instance_id AS instance_id,
                   pls.license_instance_public_key AS instance_public_key
            FROM purchase_link_session pls
            JOIN checkout_session cs ON cs.id = pls.checkout_session_id
            WHERE pls.tenant_id = $1 AND cs.tenant_id = $1
              AND pls.license_instance_id = $2
              AND pls.license_instance_public_key IS NOT NULL
              AND cs.subscription_id IS NOT NULL
            ORDER BY pls.created_at DESC
            "#,
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Text, _>(instance_id)
        .get_results(conn)
        .await
        .attach("Error while finding the purchase bindings of an instance")
        .into_db_result()
    }

    /// Purchases started before `older_than` (and within the last 30 days) whose checkout
    /// never went through, and whose customer holds no subscription, invoice, live payment
    /// or other open checkout.
    pub async fn list_abandoned(
        conn: &mut PgConn,
        older_than: NaiveDateTime,
        limit: i64,
    ) -> DbResult<Vec<AbandonedPurchaseRow>> {
        sql_query(format!(
            r#"
            SELECT pls.id, pls.tenant_id, pls.customer_id, pls.checkout_session_id
            FROM purchase_link_session pls
            LEFT JOIN checkout_session cs ON cs.id = pls.checkout_session_id
            WHERE pls.created_at < $1
              AND pls.created_at >= NOW() - INTERVAL '30 days'
              AND {ABANDONED}
            ORDER BY pls.created_at
            LIMIT $2
            "#
        ))
        .bind::<sql_types::Timestamp, _>(older_than)
        .bind::<sql_types::BigInt, _>(limit)
        .get_results(conn)
        .await
        .attach("Error while listing abandoned purchases")
        .into_db_result()
    }

    /// Re-checks a candidate of [`Self::list_abandoned`] and locks it, after its customer, for
    /// the cleanup's transaction. `None` when it no longer qualifies (resumed, paid, cleaned).
    pub async fn lock_abandoned(
        conn: &mut PgConn,
        lock: &CustomerLock,
        candidate: &AbandonedPurchaseRow,
        older_than: NaiveDateTime,
    ) -> DbResult<Option<AbandonedPurchaseRow>> {
        lock.covers(conn, candidate.tenant_id, candidate.customer_id)?;
        sql_query(format!(
            r#"
            SELECT pls.id, pls.tenant_id, pls.customer_id, pls.checkout_session_id
            FROM purchase_link_session pls
            LEFT JOIN checkout_session cs ON cs.id = pls.checkout_session_id
            WHERE pls.id = $1
              AND pls.customer_id = $2
              AND pls.checkout_session_id IS NOT DISTINCT FROM $3
              AND pls.created_at < $4
              AND {ABANDONED}
            FOR UPDATE OF pls
            "#
        ))
        .bind::<sql_types::Uuid, _>(candidate.id)
        .bind::<sql_types::Uuid, _>(lock.customer_id())
        .bind::<sql_types::Nullable<sql_types::Uuid>, _>(candidate.checkout_session_id)
        .bind::<sql_types::Timestamp, _>(older_than)
        .get_result(conn)
        .await
        .optional()
        .attach("Error while locking an abandoned purchase")
        .into_db_result()
    }

    pub async fn mark_cleaned_up(
        conn: &mut PgConn,
        ids: &[PurchaseLinkSessionId],
    ) -> DbResult<usize> {
        use crate::schema::purchase_link_session::dsl as pls;

        if ids.is_empty() {
            return Ok(0);
        }

        diesel::update(pls::purchase_link_session)
            .filter(pls::id.eq_any(ids))
            .filter(pls::cleaned_up_at.is_null())
            .set(pls::cleaned_up_at.eq(diesel::dsl::now))
            .execute(conn)
            .await
            .attach("Error while marking purchases cleaned up")
            .into_db_result()
    }
}
