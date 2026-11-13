use crate::applied_coupons::{
    AppliedCouponDetailedRow, AppliedCouponForDisplayRow, AppliedCouponRow, AppliedCouponRowNew,
};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};
use common_domain::ids::{AppliedCouponId, CouponId, CustomerId, SubscriptionId, TenantId};
use diesel::{BoolExpressionMethods, ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use rust_decimal::Decimal;
use std::collections::HashSet;

impl AppliedCouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<AppliedCouponRow> {
        use crate::schema::applied_coupon::dsl as ac;

        let query = diesel::insert_into(ac::applied_coupon).values((
            self,
            crate::schema::applied_coupon::created_at.eq(common_domain::clock::now().naive_utc()),
        ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting applied coupon")
            .into_db_result()
    }
}

impl AppliedCouponRow {
    pub async fn count_by_coupon_id(conn: &mut PgConn, param_coupon_id: CouponId) -> DbResult<i64> {
        use crate::schema::applied_coupon::dsl as ac;

        let query = ac::applied_coupon
            .filter(ac::coupon_id.eq(param_coupon_id))
            .count();

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting applied coupons by coupon id")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: Vec<&AppliedCouponRowNew>,
    ) -> DbResult<Vec<AppliedCouponRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;

        let query = diesel::insert_into(ac_dsl::applied_coupon).values(
            batch
                .iter()
                .map(|row| {
                    (
                        *row,
                        crate::schema::applied_coupon::created_at
                            .eq(common_domain::clock::now().naive_utc()),
                    )
                })
                .collect::<Vec<_>>(),
        );

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while inserting AppliedCoupon batch")
            .into_db_result()
    }

    /// Updates applied coupon state after an invoice is finalized.
    ///
    /// This function handles the SQL NULL arithmetic issue where `NULL + 1 = NULL`.
    /// We use coalesce to properly increment from NULL.
    ///
    /// Database constraints:
    /// - `applied_count IS NULL OR applied_count > 0` (can't store 0)
    /// - `applied_amount IS NULL OR applied_amount > 0` (can't store 0)
    ///
    /// Logic:
    /// - `applied_count` is always incremented (starts from NULL -> 1)
    /// - `applied_amount` is only updated when `amount_delta > 0` to avoid storing 0
    pub async fn refresh_state(
        conn: &mut PgConn,
        id: AppliedCouponId,
        amount_delta: Option<Decimal>,
    ) -> DbResult<()> {
        use crate::schema::applied_coupon::dsl as ac;
        use diesel::NullableExpressionMethods;

        diesel::define_sql_function! {
            #[sql_name = "COALESCE"]
            fn coalesce_int(x: diesel::sql_types::Nullable<diesel::sql_types::Integer>, y: diesel::sql_types::Integer) -> diesel::sql_types::Integer;
        }
        diesel::define_sql_function! {
            #[sql_name = "COALESCE"]
            fn coalesce_numeric(x: diesel::sql_types::Nullable<diesel::sql_types::Numeric>, y: diesel::sql_types::Numeric) -> diesel::sql_types::Numeric;
        }

        let now = common_domain::clock::now().naive_utc();
        let amount_delta = amount_delta.unwrap_or(Decimal::ZERO);

        if amount_delta > Decimal::ZERO {
            let query = diesel::update(ac::applied_coupon.find(id)).set((
                ac::last_applied_at.eq(Some(now)),
                ac::applied_count.eq((coalesce_int(ac::applied_count, 0) + 1).nullable()),
                ac::applied_amount.eq((coalesce_numeric(ac::applied_amount, Decimal::ZERO)
                    + amount_delta)
                    .nullable()),
            ));

            crate::debug_sql(&query);

            query
                .execute(conn)
                .await
                .attach("Error while refreshing applied coupon state")
                .into_db_result()?;
        } else {
            let query = diesel::update(ac::applied_coupon.find(id)).set((
                ac::last_applied_at.eq(Some(now)),
                ac::applied_count.eq((coalesce_int(ac::applied_count, 0) + 1).nullable()),
            ));

            crate::debug_sql(&query);

            query
                .execute(conn)
                .await
                .attach("Error while refreshing applied coupon state")
                .into_db_result()?;
        }

        Ok(())
    }

    /// Undoes what `refresh_state` recorded for an invoice that is voided: one application, and
    /// the amount it consumed when given. Zero is stored as NULL (the column constraints).
    pub async fn release_state(
        conn: &mut PgConn,
        lock: &CustomerLock,
        id: AppliedCouponId,
        amount_delta: Option<Decimal>,
    ) -> DbResult<()> {
        use diesel::sql_types::{Nullable, Numeric, Uuid};

        lock.on(conn);
        diesel::sql_query(
            "UPDATE applied_coupon SET
                 applied_count = NULLIF(GREATEST(COALESCE(applied_count, 0) - 1, 0), 0),
                 applied_amount = CASE WHEN $2 IS NULL THEN applied_amount
                     ELSE NULLIF(GREATEST(COALESCE(applied_amount, 0) - $2, 0), 0) END
             WHERE id = $1 AND customer_id = $3",
        )
        .bind::<Uuid, _>(id)
        .bind::<Nullable<Numeric>, _>(amount_delta)
        .bind::<Uuid, _>(lock.customer_id())
        .execute(conn)
        .await
        .attach("Error while releasing applied coupon state")
        .into_db_result()?;
        Ok(())
    }

    /// Whether one of the customer's redemptions of the coupon already funded a credit grant.
    pub async fn customer_redemption_funded(
        conn: &mut PgConn,
        tenant_id: TenantId,
        coupon_id: CouponId,
        customer_id: CustomerId,
        grant_source_kind: &str,
    ) -> DbResult<bool> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::customer_credit_grant::dsl as g_dsl;

        let redemptions = ac_dsl::applied_coupon
            .filter(ac_dsl::coupon_id.eq(coupon_id))
            .filter(ac_dsl::customer_id.eq(customer_id))
            .select(ac_dsl::id);
        let query = diesel::select(diesel::dsl::exists(
            g_dsl::customer_credit_grant
                .filter(g_dsl::tenant_id.eq(tenant_id))
                .filter(g_dsl::customer_id.eq(customer_id))
                .filter(g_dsl::source_kind.eq(grant_source_kind))
                .filter(g_dsl::source_id.eq_any(redemptions)),
        ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while checking a customer's funded coupon redemptions")
            .into_db_result()
    }

    /// How many redemptions of the coupon, any customer's, funded a credit grant.
    pub async fn count_funded_redemptions(
        conn: &mut PgConn,
        tenant_id: TenantId,
        coupon_id: CouponId,
        grant_source_kind: &str,
    ) -> DbResult<i64> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::customer_credit_grant::dsl as g_dsl;

        let redemptions = ac_dsl::applied_coupon
            .filter(ac_dsl::coupon_id.eq(coupon_id))
            .select(ac_dsl::id);
        let query = g_dsl::customer_credit_grant
            .filter(g_dsl::tenant_id.eq(tenant_id))
            .filter(g_dsl::source_kind.eq(grant_source_kind))
            .filter(g_dsl::source_id.eq_any(redemptions))
            .count();

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting a coupon's funded redemptions")
            .into_db_result()
    }

    /// Marks a redemption that will never fund its credit as inactive.
    pub async fn deactivate(conn: &mut PgConn, id: AppliedCouponId) -> DbResult<()> {
        use crate::schema::applied_coupon::dsl as ac_dsl;

        diesel::update(ac_dsl::applied_coupon.filter(ac_dsl::id.eq(id)))
            .set(ac_dsl::is_active.eq(false))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while deactivating an applied coupon")
            .into_db_result()
    }

    /// Returns the set of (coupon_id, customer_id) pairs from the input that already exist in the database
    pub async fn find_existing_customer_coupon_pairs(
        conn: &mut PgConn,
        pairs: &[(CouponId, CustomerId)],
    ) -> DbResult<HashSet<(CouponId, CustomerId)>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;

        if pairs.is_empty() {
            return Ok(HashSet::new());
        }

        let mut query = ac_dsl::applied_coupon.into_boxed();

        for (coupon_id, customer_id) in pairs {
            query = query.or_filter(
                ac_dsl::coupon_id
                    .eq(*coupon_id)
                    .and(ac_dsl::customer_id.eq(*customer_id)),
            );
        }

        let results: Vec<(CouponId, CustomerId)> = query
            .select((ac_dsl::coupon_id, ac_dsl::customer_id))
            .load(conn)
            .await
            .attach("Error while checking existing customer-coupon pairs")
            .into_db_result()?;

        log::debug!("Found {} existing customer-coupon pairs", results.len());

        Ok(results.into_iter().collect())
    }
}

impl AppliedCouponForDisplayRow {
    pub async fn list_by_coupon_id(
        conn: &mut PgConn,
        param_coupon_id: &CouponId,
        tenant_id: &TenantId,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<AppliedCouponForDisplayRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::coupon::dsl as cou_dsl;
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::subscription::dsl as s_dsl;

        let query = ac_dsl::applied_coupon
            .inner_join(cou_dsl::coupon)
            .inner_join(c_dsl::customer)
            .inner_join(s_dsl::subscription)
            .inner_join(pv_dsl::plan_version.on(s_dsl::plan_version_id.eq(pv_dsl::id)))
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(cou_dsl::id.eq(param_coupon_id))
            .filter(cou_dsl::tenant_id.eq(tenant_id))
            .order(ac_dsl::created_at.desc())
            .select(AppliedCouponForDisplayRow::as_select());

        let paginated_query = query.paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing applied coupons by coupon id")
            .into_db_result()
    }
}

impl AppliedCouponDetailedRow {
    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        param_subscription_id: &SubscriptionId,
    ) -> DbResult<Vec<AppliedCouponDetailedRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::coupon::dsl as c_dsl;

        let query = ac_dsl::applied_coupon
            .inner_join(c_dsl::coupon)
            .filter(ac_dsl::subscription_id.eq(param_subscription_id))
            .select(AppliedCouponDetailedRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing applied coupons by subscription id")
            .into_db_result()
    }

    /// Locks the applied coupons and their coupons, in coupon id order: a coupon is shared by
    /// the tenant's customers, so two of them must lock the coupons they share in one order.
    pub async fn list_by_ids_for_update(
        conn: &mut PgConn,
        lock: &CustomerLock,
        applied_coupons_ids: &[AppliedCouponId],
    ) -> DbResult<Vec<AppliedCouponDetailedRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::coupon::dsl as c_dsl;

        let query = ac_dsl::applied_coupon
            .inner_join(c_dsl::coupon)
            .filter(ac_dsl::id.eq_any(applied_coupons_ids))
            .order((c_dsl::id, ac_dsl::id))
            .select(AppliedCouponDetailedRow::as_select())
            .for_update();

        crate::debug_sql(&query);

        let rows: Vec<AppliedCouponDetailedRow> = query
            .get_results(conn)
            .await
            .attach("Error while listing applied coupons by applied_coupon_id")
            .into_db_result()?;
        lock.on(conn);
        rows.iter().try_for_each(|row| {
            lock.covers(conn, row.coupon.tenant_id, row.applied_coupon.customer_id)
        })?;
        Ok(rows)
    }
}
