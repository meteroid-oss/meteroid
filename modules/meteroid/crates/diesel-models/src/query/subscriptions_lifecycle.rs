use crate::errors::IntoDbResult;
use chrono::NaiveDate;

use crate::subscriptions::{
    SubscriptionCycleErrorRowPatch, SubscriptionCycleRowPatch, SubscriptionRow,
};
use crate::{DbResult, PgConn};

use diesel::dsl::not;
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, debug_query};
use diesel_async::RunQueryDsl;

use crate::enums::{CycleActionEnum, SubscriptionStatusEnum};
use common_domain::ids::{SubscriptionId, TenantId};
use error_stack::ResultExt;
use uuid::Uuid;

impl SubscriptionRow {
    #[allow(clippy::too_many_arguments)]
    pub async fn activate_subscription(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
        next_cycle_action: Option<CycleActionEnum>,
        cycle_index: Option<i32>,
        status: SubscriptionStatusEnum,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::activated_at.is_null())
            .set((
                s_dsl::activated_at.eq(chrono::Utc::now().naive_utc()),
                s_dsl::current_period_start.eq(current_period_start),
                s_dsl::current_period_end.eq(current_period_end),
                s_dsl::next_cycle_action.eq(next_cycle_action),
                s_dsl::cycle_index.eq(cycle_index),
                s_dsl::status.eq(status),
                s_dsl::pending_checkout.eq(false),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while activating subscription")
            .into_db_result()?;

        Ok(())
    }

    /// Activates a subscription and sets the payment method in one operation.
    /// Used for free trial checkout where we need to save the card for future billing.
    #[allow(clippy::too_many_arguments)]
    pub async fn activate_subscription_with_payment_method(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
        next_cycle_action: Option<CycleActionEnum>,
        cycle_index: Option<i32>,
        status: SubscriptionStatusEnum,
        payment_method: Option<common_domain::ids::CustomerPaymentMethodId>,
        payment_method_type: Option<crate::enums::PaymentMethodTypeEnum>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::activated_at.is_null())
            .set((
                s_dsl::activated_at.eq(chrono::Utc::now().naive_utc()),
                s_dsl::current_period_start.eq(current_period_start),
                s_dsl::current_period_end.eq(current_period_end),
                s_dsl::next_cycle_action.eq(next_cycle_action),
                s_dsl::cycle_index.eq(cycle_index),
                s_dsl::status.eq(status),
                s_dsl::pending_checkout.eq(false),
                s_dsl::payment_method.eq(payment_method),
                s_dsl::payment_method_type.eq(payment_method_type),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while activating subscription with payment method")
            .into_db_result()?;

        Ok(())
    }

    /// Transitions a TrialExpired subscription to Active after payment.
    /// Unlike activate_subscription, this works on already-activated subscriptions.
    #[allow(clippy::too_many_arguments)]
    pub async fn transition_trial_expired_to_active(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
        next_cycle_action: Option<CycleActionEnum>,
        cycle_index: Option<i32>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::status.eq(SubscriptionStatusEnum::TrialExpired))
            .set((
                s_dsl::current_period_start.eq(current_period_start),
                s_dsl::current_period_end.eq(current_period_end),
                s_dsl::next_cycle_action.eq(next_cycle_action),
                s_dsl::cycle_index.eq(cycle_index),
                s_dsl::status.eq(SubscriptionStatusEnum::Active),
                s_dsl::pending_checkout.eq(false),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while transitioning subscription from TrialExpired to Active")
            .into_db_result()?;

        Ok(())
    }

    pub async fn update_subscription_mrr_delta(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        mrr_cents_delta: i64,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::{id, mrr_cents, subscription};

        let query = diesel::update(subscription)
            .filter(id.eq(subscription_id))
            .set(mrr_cents.eq(mrr_cents + mrr_cents_delta));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription MRR")
            .into_db_result()?;

        Ok(())
    }

    pub async fn update_subscription_payment_method(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        payment_method_id: Option<common_domain::ids::CustomerPaymentMethodId>,
        payment_method_type: Option<crate::enums::PaymentMethodTypeEnum>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(subscription_id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .set((
                dsl::payment_method.eq(payment_method_id),
                dsl::payment_method_type.eq(payment_method_type),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription payment method")
            .into_db_result()?;

        Ok(())
    }

    pub async fn lock_subscription_for_update(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::{id, subscription};

        let query = subscription
            .for_update()
            .select(id)
            .filter(id.eq(subscription_id_param));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let _res: Uuid = query
            .get_result(conn)
            .await
            .attach("Error while locking subscription for update")
            .into_db_result()?;

        Ok(())
    }

    pub async fn get_due_subscription_for_update(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<SubscriptionRow>> {
        use crate::schema::subscription::dsl;

        let now = chrono::Utc::now().naive_utc();

        let query = dsl::subscription
            .filter(dsl::current_period_end.le(now.date()))
            // Only process if next_retry is null (no error) or next_retry time has passed
            .filter(dsl::next_retry.is_null().or(dsl::next_retry.le(now)))
            .filter(not(dsl::status.eq_any(vec![
                SubscriptionStatusEnum::Cancelled,
                SubscriptionStatusEnum::Completed,
                SubscriptionStatusEnum::Superseded,
                SubscriptionStatusEnum::Suspended,
                SubscriptionStatusEnum::Errored,
            ])))
            .order_by(dsl::current_period_end.asc())
            .limit(limit)
            .for_update()
            .skip_locked();

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let res = query
            .get_results(conn)
            .await
            .attach("Error while fetching due subscriptions")
            .into_db_result()?;

        Ok(res)
    }
}

impl SubscriptionCycleRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(&self.id))
            .filter(dsl::tenant_id.eq(&self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription cycles")
            .into_db_result()?;

        Ok(())
    }
}
impl SubscriptionCycleErrorRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(&self.id))
            .filter(dsl::tenant_id.eq(&self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription errors")
            .into_db_result()?;

        Ok(())
    }
}
