use crate::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use crate::enums::{CheckoutSessionStatusEnum, PaymentStatusEnum};
use crate::errors::IntoDbResult;
use crate::lock::CustomerLock;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{CheckoutSessionId, CustomerId, SubscriptionId, TenantId};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, NullableExpressionMethods, OptionalExtension,
    QueryDsl,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl CheckoutSessionRowNew {
    pub async fn insert_or_get(&self, conn: &mut PgConn) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs;
        diesel::insert_into(cs::checkout_session)
            .values(self)
            .on_conflict(cs::id)
            .do_nothing()
            .execute(conn)
            .await
            .attach("Error creating checkout session")
            .into_db_result()?;
        CheckoutSessionRow::get_by_id(conn, self.tenant_id, self.id).await
    }

    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::insert_into(cs_dsl::checkout_session).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting checkout session")
            .into_db_result()
    }
}

impl CheckoutSessionRow {
    pub async fn complete_top_up(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<()> {
        use crate::schema::checkout_session::dsl as cs;
        diesel::update(
            cs::checkout_session
                .filter(cs::tenant_id.eq(tenant_id))
                .filter(cs::id.eq(id)),
        )
        .set((
            cs::status.eq(CheckoutSessionStatusEnum::Completed),
            cs::completed_at.eq(Utc::now()),
        ))
        .execute(conn)
        .await
        .attach("Error completing top-up checkout")
        .into_db_result()?;
        Ok(())
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<Option<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        cs_dsl::checkout_session
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .first(conn)
            .await
            .optional()
            .attach("Error while finding checkout session")
            .into_db_result()
    }

    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = cs_dsl::checkout_session
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while getting checkout session")
            .into_db_result()
    }

    /// Locks the session's customer, then the session (`crate::lock`). NO KEY UPDATE: a payment
    /// recorded against the session (KEY SHARE) does not wait on it.
    pub async fn get_by_id_for_update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<(CustomerLock, CheckoutSessionRow)> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let customer_id: CustomerId = cs_dsl::checkout_session
            .select(cs_dsl::customer_id)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .first(conn)
            .await
            .attach("Error while reading the customer of a checkout session")
            .into_db_result()?;
        let lock = crate::customers::CustomerRow::lock(conn, tenant_id, customer_id).await?;

        let query = cs_dsl::checkout_session
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .for_no_key_update();

        crate::debug_sql(&query);

        let row: CheckoutSessionRow = query
            .first(conn)
            .await
            .attach("Error while getting checkout session for update")
            .into_db_result()?;
        lock.covers(conn, row.tenant_id, row.customer_id)?;
        Ok((lock, row))
    }

    /// Returns the most recent session that is still in Created or AwaitingPayment status.
    pub async fn get_by_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> DbResult<Option<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = cs_dsl::checkout_session
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(cs_dsl::subscription_id.eq(subscription_id))
            .filter(
                cs_dsl::status
                    .eq(CheckoutSessionStatusEnum::Created)
                    .or(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment)),
            )
            .order_by(cs_dsl::created_at.desc());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while getting checkout session by subscription")
            .into_db_result()
    }

    pub async fn mark_completed(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
        subscription_id: SubscriptionId,
        completed_at: DateTime<Utc>,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .set((
                cs_dsl::status.eq(CheckoutSessionStatusEnum::Completed),
                cs_dsl::subscription_id.eq(subscription_id),
                cs_dsl::completed_at.eq(completed_at),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while marking checkout session as completed")
            .into_db_result()
    }

    /// Persist the coupons a checkout payment is priced with, so the deferred materialization
    /// (an asynchronous settlement, `billing_requests.fulfilled`) rebuilds the subscription with
    /// the SAME coupons the payment amount was computed from. The coupons before it are kept (the
    /// first attempt's, if an older one never restored them) for `reopen_after_failed_payment`.
    pub async fn set_payment_coupons(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
        coupon_code: Option<String>,
        coupon_ids: Vec<Option<common_domain::ids::CouponId>>,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;
        use diesel::sql_types::{Array, Nullable, Uuid, Varchar};

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .set((
                cs_dsl::pre_payment_coupon_code.eq(diesel::dsl::sql::<Nullable<Varchar>>(
                    "CASE WHEN pre_payment_coupon_ids IS NULL THEN coupon_code ELSE pre_payment_coupon_code END",
                )),
                cs_dsl::pre_payment_coupon_ids.eq(diesel::dsl::sql::<
                    Nullable<Array<Nullable<Uuid>>>,
                >(
                    "COALESCE(pre_payment_coupon_ids, coupon_ids)"
                )),
                cs_dsl::coupon_code.eq(coupon_code),
                cs_dsl::coupon_ids.eq(coupon_ids),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while persisting checkout payment coupons")
            .into_db_result()
    }

    /// A plan-change checkout of the subscription (other than `except`) awaiting its payment.
    pub async fn plan_change_awaiting_payment(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        except: Option<CheckoutSessionId>,
    ) -> DbResult<Option<CheckoutSessionId>> {
        use crate::enums::CheckoutTypeEnum;
        use crate::schema::checkout_session::dsl as cs_dsl;

        let mut query = cs_dsl::checkout_session
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(cs_dsl::subscription_id.eq(subscription_id))
            .filter(cs_dsl::checkout_type.eq(CheckoutTypeEnum::PlanChange))
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment))
            .select(cs_dsl::id)
            .into_boxed();
        if let Some(except) = except {
            query = query.filter(cs_dsl::id.ne(except));
        }
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while looking for a plan change awaiting its payment")
            .into_db_result()
    }

    /// Expires the plan-change checkouts whose payment has waited on the customer (3DS, a
    /// redirect) since before `stale_before`: their change would apply on stale prices. A payment
    /// processing without the customer (a SEPA debit) is never abandoned: the provider ends it.
    pub async fn expire_stale_plan_change_checkouts(
        conn: &mut PgConn,
        stale_before: DateTime<Utc>,
    ) -> DbResult<usize> {
        use crate::enums::CheckoutTypeEnum;
        use crate::schema::checkout_session::dsl as cs_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;

        let waiting_on_customer = pt_dsl::payment_transaction
            .filter(pt_dsl::checkout_session_id.eq(cs_dsl::id.nullable()))
            .filter(pt_dsl::status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready]))
            .filter(pt_dsl::next_action.is_not_null())
            .filter(pt_dsl::created_at.lt(stale_before));
        let still_fresh = pt_dsl::payment_transaction
            .filter(pt_dsl::checkout_session_id.eq(cs_dsl::id.nullable()))
            .filter(pt_dsl::status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready]))
            .filter(
                pt_dsl::next_action
                    .is_null()
                    .or(pt_dsl::created_at.ge(stale_before)),
            );
        diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::checkout_type.eq(CheckoutTypeEnum::PlanChange))
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment))
            .filter(diesel::dsl::exists(waiting_on_customer))
            .filter(diesel::dsl::not(diesel::dsl::exists(still_fresh)))
            .set(cs_dsl::status.eq(CheckoutSessionStatusEnum::Expired))
            .execute(conn)
            .await
            .attach("Error while expiring stale plan-change checkouts")
            .into_db_result()
    }

    pub async fn mark_awaiting_payment(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
        subscription_id: Option<SubscriptionId>,
    ) -> DbResult<CheckoutSessionRow> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .set((
                cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment),
                cs_dsl::subscription_id.eq(subscription_id),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while marking checkout session as awaiting payment")
            .into_db_result()
    }

    /// Returns the session to `Created` after a failed hosted attempt so it can be retried.
    /// Stays `AwaitingPayment` while any transaction is non-terminal (a retry is in flight).
    /// Checkouts whose payment is still settling and that hold `coupon_id`, optionally of one
    /// customer: the redemption they will make once it settles.
    pub async fn count_awaiting_payment_with_coupon(
        conn: &mut PgConn,
        tenant_id: TenantId,
        coupon_id: common_domain::ids::CouponId,
        customer_id: Option<CustomerId>,
    ) -> DbResult<i64> {
        use crate::schema::checkout_session::dsl as cs_dsl;
        use diesel::PgArrayExpressionMethods;

        let mut query = cs_dsl::checkout_session
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment))
            .filter(cs_dsl::coupon_ids.contains(vec![Some(coupon_id)]))
            .into_boxed();
        if let Some(customer_id) = customer_id {
            query = query.filter(cs_dsl::customer_id.eq(customer_id));
        }
        let query = query.count();

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting checkouts awaiting payment with a coupon")
            .into_db_result()
    }

    pub async fn reopen_after_failed_payment(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<Option<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;

        let in_flight = pt_dsl::payment_transaction
            .filter(pt_dsl::checkout_session_id.eq(id))
            .filter(pt_dsl::tenant_id.eq(tenant_id))
            .filter(pt_dsl::status.eq_any([PaymentStatusEnum::Pending, PaymentStatusEnum::Ready]));

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment))
            .filter(diesel::dsl::not(diesel::dsl::exists(in_flight)))
            .set((
                cs_dsl::status.eq(CheckoutSessionStatusEnum::Created),
                // The failed attempt's coupons leave with it.
                cs_dsl::coupon_code.eq(diesel::dsl::sql::<diesel::sql_types::Nullable<diesel::sql_types::Varchar>>(
                    "CASE WHEN pre_payment_coupon_ids IS NULL THEN coupon_code ELSE pre_payment_coupon_code END",
                )),
                cs_dsl::coupon_ids.eq(diesel::dsl::sql::<
                    diesel::sql_types::Array<diesel::sql_types::Nullable<diesel::sql_types::Uuid>>,
                >("COALESCE(pre_payment_coupon_ids, coupon_ids)")),
                cs_dsl::pre_payment_coupon_code.eq(None::<String>),
                cs_dsl::pre_payment_coupon_ids.eq(None::<Vec<Option<common_domain::ids::CouponId>>>),
            ));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while reopening checkout session after a failed payment")
            .into_db_result()
    }

    /// Expire ONE session, only from a non-terminal state. 0 rows updated
    /// means it already reached a terminal state — the guard for races with a
    /// concurrent completion.
    pub async fn mark_expired_single(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<usize> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(
                cs_dsl::status
                    .eq(CheckoutSessionStatusEnum::Created)
                    .or(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment)),
            )
            .set(cs_dsl::status.eq(CheckoutSessionStatusEnum::Expired));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while expiring checkout session")
            .into_db_result()
    }

    pub async fn mark_expired_batch(conn: &mut PgConn, now: DateTime<Utc>) -> DbResult<usize> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::Created))
            .filter(cs_dsl::expires_at.is_not_null())
            .filter(cs_dsl::expires_at.lt(now))
            .set(cs_dsl::status.eq(CheckoutSessionStatusEnum::Expired));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while marking expired checkout sessions")
            .into_db_result()
    }

    /// Delete old expired checkout sessions after retention period.
    /// Sessions with payment transactions are kept: they anchor the payment history.
    pub async fn delete_old(conn: &mut PgConn, older_than: DateTime<Utc>) -> DbResult<usize> {
        use crate::schema::checkout_session::dsl as cs_dsl;
        use crate::schema::payment_transaction::dsl as pt_dsl;

        let query = diesel::delete(cs_dsl::checkout_session)
            .filter(cs_dsl::status.eq(CheckoutSessionStatusEnum::Expired))
            .filter(cs_dsl::created_at.lt(older_than))
            .filter(diesel::dsl::not(diesel::dsl::exists(
                pt_dsl::payment_transaction
                    .filter(pt_dsl::checkout_session_id.eq(cs_dsl::id.nullable()))
                    .select(diesel::dsl::sql::<diesel::sql_types::Integer>("1")),
            )));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting old checkout sessions")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<CheckoutSessionStatusEnum>,
    ) -> DbResult<Vec<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let mut query = cs_dsl::checkout_session
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .order_by(cs_dsl::created_at.desc())
            .into_boxed();

        if let Some(cid) = customer_id {
            query = query.filter(cs_dsl::customer_id.eq(cid));
        }

        if let Some(s) = status {
            query = query.filter(cs_dsl::status.eq(s));
        }

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing checkout sessions")
            .into_db_result()
    }

    pub async fn mark_cancelled(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CheckoutSessionId,
    ) -> DbResult<Option<CheckoutSessionRow>> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        // Allow cancellation of Created or AwaitingPayment sessions
        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::id.eq(id))
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(
                cs_dsl::status
                    .eq(CheckoutSessionStatusEnum::Created)
                    .or(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment)),
            )
            .set(cs_dsl::status.eq(CheckoutSessionStatusEnum::Cancelled));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while cancelling checkout session")
            .into_db_result()
    }

    /// Cancel every still-open (Created / AwaitingPayment) checkout session for a subscription,
    /// so an aborted subscription can no longer be paid for and resurrected.
    pub async fn cancel_open_for_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<usize> {
        use crate::schema::checkout_session::dsl as cs_dsl;

        let query = diesel::update(cs_dsl::checkout_session)
            .filter(cs_dsl::tenant_id.eq(tenant_id))
            .filter(cs_dsl::subscription_id.eq(subscription_id_param))
            .filter(
                cs_dsl::status
                    .eq(CheckoutSessionStatusEnum::Created)
                    .or(cs_dsl::status.eq(CheckoutSessionStatusEnum::AwaitingPayment)),
            )
            .set(cs_dsl::status.eq(CheckoutSessionStatusEnum::Cancelled));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while cancelling open checkout sessions for subscription")
            .into_db_result()
    }
}
