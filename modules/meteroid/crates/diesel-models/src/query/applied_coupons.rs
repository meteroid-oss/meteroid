use crate::applied_coupons::{
    AppliedCouponDetailedRow, AppliedCouponForDisplayRow, AppliedCouponRow, AppliedCouponRowNew,
};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::{DbResult, PgConn};
use common_domain::ids::{AppliedCouponId, BaseId, CouponId, CustomerId, SubscriptionId, TenantId};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, JoinOnDsl, QueryDsl, SelectableHelper, debug_query,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use rust_decimal::Decimal;
use std::collections::HashSet;

impl AppliedCouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<AppliedCouponRow> {
        use crate::schema::applied_coupon::dsl as ac;

        let query = diesel::insert_into(ac::applied_coupon).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

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

        let query = diesel::insert_into(ac_dsl::applied_coupon).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting AppliedCoupon batch")
            .into_db_result()
    }

    /// Updates applied coupon state after an invoice is finalized.
    ///
    /// This function handles the SQL NULL arithmetic issue where `NULL + 1 = NULL`.
    /// We use COALESCE in raw SQL to properly increment from NULL.
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
        use diesel::sql_query;
        use diesel::sql_types::{Nullable, Numeric, Timestamp, Uuid as DieselUuid};

        let now = chrono::Utc::now().naive_utc();
        let amount_delta = amount_delta.unwrap_or(Decimal::ZERO);

        if amount_delta > Decimal::ZERO {
            let query = sql_query(
                "UPDATE applied_coupon SET \
                 last_applied_at = $1, \
                 applied_count = COALESCE(applied_count, 0) + 1, \
                 applied_amount = COALESCE(applied_amount, 0) + $2 \
                 WHERE id = $3",
            )
            .bind::<Nullable<Timestamp>, _>(Some(now))
            .bind::<Numeric, _>(amount_delta)
            .bind::<DieselUuid, _>(id.as_uuid());

            log::debug!("{:?}", query);

            query
                .execute(conn)
                .await
                .attach("Error while refreshing applied coupon state")
                .into_db_result()?;
        } else {
            let query = sql_query(
                "UPDATE applied_coupon SET \
                 last_applied_at = $1, \
                 applied_count = COALESCE(applied_count, 0) + 1 \
                 WHERE id = $2",
            )
            .bind::<Nullable<Timestamp>, _>(Some(now))
            .bind::<DieselUuid, _>(id.as_uuid());

            log::debug!("{:?}", query);

            query
                .execute(conn)
                .await
                .attach("Error while refreshing applied coupon state")
                .into_db_result()?;
        }

        Ok(())
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing applied coupons by subscription id")
            .into_db_result()
    }

    pub async fn list_by_ids_for_update(
        conn: &mut PgConn,
        applied_coupons_ids: &[AppliedCouponId],
    ) -> DbResult<Vec<AppliedCouponDetailedRow>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;
        use crate::schema::coupon::dsl as c_dsl;

        let query = ac_dsl::applied_coupon
            .inner_join(c_dsl::coupon)
            .filter(ac_dsl::id.eq_any(applied_coupons_ids))
            .select(AppliedCouponDetailedRow::as_select())
            .for_update();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing applied coupons by applied_coupon_id")
            .into_db_result()
    }
}
