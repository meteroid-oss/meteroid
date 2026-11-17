use crate::errors::IntoDbResult;
use chrono::NaiveDate;

use crate::lock::CustomerLock;
use crate::subscriptions::{
    SubscriptionCadenceRowPatch, SubscriptionCycleRowPatch, SubscriptionRow,
};
use crate::{DbResult, PgConn};

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;

use crate::enums::SubscriptionStatusEnum;
use common_domain::ids::{CustomerId, PlanVersionId, SubscriptionId, TenantId};
use error_stack::ResultExt;

impl SubscriptionRow {
    pub async fn activate_subscription(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
        status: SubscriptionStatusEnum,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::activated_at.is_null())
            .set((
                s_dsl::activated_at.eq(common_clock::now().naive_utc()),
                s_dsl::current_period_start.eq(current_period_start),
                s_dsl::current_period_end.eq(current_period_end),
                s_dsl::status.eq(status),
                s_dsl::pending_checkout.eq(false),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while activating subscription")
            .into_db_result()?;

        Ok(())
    }

    /// Transitions a TrialExpired subscription to Active after payment.
    /// Unlike activate_subscription, this works on already-activated subscriptions.
    pub async fn transition_trial_expired_to_active(
        conn: &mut PgConn,
        id: &SubscriptionId,
        tenant_id: &TenantId,
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        let query = diesel::update(s_dsl::subscription)
            .filter(s_dsl::id.eq(id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::status.eq(SubscriptionStatusEnum::TrialExpired))
            .set((
                s_dsl::current_period_start.eq(current_period_start),
                s_dsl::current_period_end.eq(current_period_end),
                s_dsl::status.eq(SubscriptionStatusEnum::Active),
                s_dsl::pending_checkout.eq(false),
            ));

        crate::debug_sql(&query);

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

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription MRR")
            .into_db_result()?;

        Ok(())
    }

    pub async fn get_tax_inclusive_rate(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        subscription_id: SubscriptionId,
    ) -> DbResult<Option<rust_decimal::Decimal>> {
        use crate::schema::subscription::dsl::{id, subscription, tax_inclusive_rate, tenant_id};

        subscription
            .filter(id.eq(subscription_id))
            .filter(tenant_id.eq(tenant_id_param))
            .select(tax_inclusive_rate)
            .first(conn)
            .await
            .attach("Error while reading the subscription's tax-inclusive rate")
            .into_db_result()
    }

    pub async fn update_tax_inclusive_rate(
        conn: &mut PgConn,
        tenant_id_param: TenantId,
        subscription_id: SubscriptionId,
        rate: Option<rust_decimal::Decimal>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::{id, subscription, tax_inclusive_rate, tenant_id};

        diesel::update(subscription)
            .filter(id.eq(subscription_id))
            .filter(tenant_id.eq(tenant_id_param))
            .set(tax_inclusive_rate.eq(rate))
            .execute(conn)
            .await
            .attach("Error while updating the subscription's tax-inclusive rate")
            .into_db_result()?;

        Ok(())
    }

    /// Locks the customer of the subscription, then the subscription (`crate::lock`).
    pub async fn lock_subscription_for_update(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<CustomerLock> {
        let lock = Self::lock_customer_of(conn, subscription_id_param).await?;
        Self::lock_subscription(conn, &lock, subscription_id_param).await?;
        Ok(lock)
    }

    pub async fn lock_subscription(
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl::{customer_id, id, subscription, tenant_id};

        // NO KEY UPDATE: inserts referencing the subscription (events, invoices, seats) take KEY
        // SHARE on it, which FOR UPDATE would block; no path deletes it or changes its key.
        let query = subscription
            .for_no_key_update()
            .select((tenant_id, customer_id))
            .filter(id.eq(subscription_id_param));

        crate::debug_sql(&query);

        let (tenant, customer): (TenantId, CustomerId) = query
            .get_result(conn)
            .await
            .attach("Error while locking subscription for update")
            .into_db_result()?;
        lock.covers(conn, tenant, customer)
    }

    pub async fn lock_customer_of(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<CustomerLock> {
        use crate::schema::subscription::dsl::{customer_id, id, subscription, tenant_id};

        let (customer, tenant): (CustomerId, TenantId) = subscription
            .select((customer_id, tenant_id))
            .filter(id.eq(subscription_id_param))
            .first(conn)
            .await
            .attach("Error while reading the customer of a subscription")
            .into_db_result()?;
        crate::customers::CustomerRow::lock(conn, tenant, customer).await
    }

    /// `lock_customer_of`, for a subscription of `tenant` only.
    pub async fn lock_customer_of_tenant(
        conn: &mut PgConn,
        tenant: TenantId,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<CustomerLock> {
        use crate::schema::subscription::dsl::{customer_id, id, subscription, tenant_id};

        let customer: CustomerId = subscription
            .select(customer_id)
            .filter(id.eq(subscription_id_param))
            .filter(tenant_id.eq(tenant))
            .first(conn)
            .await
            .attach("Error while reading the customer of a subscription")
            .into_db_result()?;
        crate::customers::CustomerRow::lock(conn, tenant, customer).await
    }

    pub async fn update_plan_version(
        conn: &mut PgConn,
        subscription_id: &SubscriptionId,
        tenant_id: &TenantId,
        new_plan_version_id: PlanVersionId,
        new_period: Option<crate::enums::BillingPeriodEnum>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s_dsl;

        if let Some(period) = new_period {
            let query = diesel::update(s_dsl::subscription)
                .filter(s_dsl::id.eq(subscription_id))
                .filter(s_dsl::tenant_id.eq(tenant_id))
                .set((
                    s_dsl::plan_version_id.eq(new_plan_version_id),
                    s_dsl::period.eq(period),
                ));

            crate::debug_sql(&query);

            query
                .execute(conn)
                .await
                .attach("Error while updating subscription plan version")
                .into_db_result()?;
        } else {
            let query = diesel::update(s_dsl::subscription)
                .filter(s_dsl::id.eq(subscription_id))
                .filter(s_dsl::tenant_id.eq(tenant_id))
                .set(s_dsl::plan_version_id.eq(new_plan_version_id));

            crate::debug_sql(&query);

            query
                .execute(conn)
                .await
                .attach("Error while updating subscription plan version")
                .into_db_result()?;
        }

        Ok(())
    }
}

impl SubscriptionCycleRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(&self.id))
            .filter(dsl::tenant_id.eq(&self.tenant_id))
            .set(self);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription cycles")
            .into_db_result()?;

        Ok(())
    }
}
impl SubscriptionCadenceRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(&self.id))
            .filter(dsl::tenant_id.eq(&self.tenant_id))
            .set(self);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating subscription cadence")
            .into_db_result()?;

        Ok(())
    }
}
