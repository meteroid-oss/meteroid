use crate::errors::IntoDbResult;
use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};

use crate::lock::CustomerLock;
use crate::subscriptions::{
    SubscriptionCadenceRowPatch, SubscriptionCycleErrorRowPatch, SubscriptionCycleRowPatch,
    SubscriptionRow,
};
use crate::{DbResult, PgConn};

use diesel::dsl::sql;
use diesel::sql_types::Bool;
use diesel::{BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;

use crate::enums::{CycleActionEnum, SubscriptionStatusEnum, TenantStatusEnum};
use common_domain::ids::{CustomerId, PlanVersionId, SubscriptionId, TenantId};
use error_stack::ResultExt;

/// How long before a claim expires and can be picked up by another worker.
const CLAIM_TIMEOUT_MINUTES: i64 = 5;

/// `SubscriptionStatusEnum::not_terminal()`, inlined rather than bound so the planner can prove
/// the predicate of the partial index `idx_subscription_lifecycle_claim`.
const NOT_TERMINAL_STATUS_SQL: &str = "subscription.status IN \
     ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED')";

impl SubscriptionRow {
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

    /// Locks the customer of the subscription unless another transaction holds it (None).
    pub async fn try_lock_customer_of(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
    ) -> DbResult<Option<CustomerLock>> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::subscription::dsl::{customer_id, id, subscription, tenant_id};

        let (customer, tenant): (CustomerId, TenantId) = subscription
            .select((customer_id, tenant_id))
            .filter(id.eq(subscription_id_param))
            .first(conn)
            .await
            .attach("Error while reading the customer of a subscription")
            .into_db_result()?;
        let locked: Option<CustomerId> = c_dsl::customer
            .select(c_dsl::id)
            .filter(c_dsl::id.eq(customer))
            .filter(c_dsl::tenant_id.eq(tenant))
            .for_no_key_update()
            .skip_locked()
            .first(conn)
            .await
            .optional()
            .attach("Error while trying to lock the customer of a subscription")
            .into_db_result()?;
        Ok(locked.map(|customer| CustomerLock::taken(conn, tenant, customer)))
    }

    /// Gives a claimed subscription back to the claims due from `retry_at`, without counting an
    /// error, unless another transaction holds it (its claim then times out).
    pub async fn defer_claim_unless_locked(
        conn: &mut PgConn,
        subscription_id_param: SubscriptionId,
        retry_at: NaiveDateTime,
    ) -> DbResult<usize> {
        diesel::sql_query(
            "UPDATE subscription SET processing_started_at = NULL, next_retry = $2 \
             WHERE id = (SELECT id FROM subscription WHERE id = $1 FOR NO KEY UPDATE SKIP LOCKED)",
        )
        .bind::<diesel::sql_types::Uuid, _>(subscription_id_param)
        .bind::<diesel::sql_types::Timestamp, _>(retry_at)
        .execute(conn)
        .await
        .attach("Error while deferring a subscription claim")
        .into_db_result()
    }

    /// Claims subscription IDs for processing.
    ///
    /// Subscriptions are available if:
    /// - They are due (current_period_end <= today)
    /// - Not in a retry backoff period
    /// - Not in a terminal status
    /// - Not currently claimed (processing_started_at is null or expired)
    pub async fn claim_due_subscriptions(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<(SubscriptionId, common_domain::ids::CustomerId, TenantId)>> {
        use crate::schema::subscription::dsl;

        let now = Utc::now().naive_utc();
        let claim_timeout = now - Duration::minutes(CLAIM_TIMEOUT_MINUTES);

        // Paused/archived tenants get no scheduled lifecycle processing. `lifecycle_suspended`
        // keeps their rows out of the partial index; NOT EXISTS stays the authority (anti-join on
        // tenant_pkey) for rows whose flag is stale.
        let tenant_inactive = {
            use crate::schema::tenant::dsl as t_dsl;
            diesel::dsl::exists(
                t_dsl::tenant
                    .filter(t_dsl::id.eq(dsl::tenant_id))
                    .filter(t_dsl::status.ne(TenantStatusEnum::Active)),
            )
        };
        // A sandbox still being seeded advances its subscriptions itself (a replay of its history).
        // NOT EXISTS plans as an anti-join on sandbox_seed's key; `<> ALL (subquery)` re-scans per row.
        let held_tenant = {
            use crate::schema::sandbox_seed::dsl as s_dsl;
            s_dsl::sandbox_seed
                .filter(s_dsl::tenant_id.eq(dsl::tenant_id))
                .filter(s_dsl::status.eq(crate::enums::SandboxSeedStatusEnum::InProgress))
                .select(diesel::dsl::sql::<diesel::sql_types::Integer>("1"))
        };

        // Step 1: Find and lock available subscriptions (prevents concurrent claims)
        let select_query = dsl::subscription
            .select((dsl::id, dsl::customer_id, dsl::tenant_id))
            .filter(dsl::current_period_end.le(now.date()))
            // Only process if next_retry is null (no error) or next_retry time has passed
            .filter(dsl::next_retry.is_null().or(dsl::next_retry.le(now)))
            // Not currently claimed (null or expired claim)
            .filter(
                dsl::processing_started_at
                    .is_null()
                    .or(dsl::processing_started_at.lt(claim_timeout)),
            )
            .filter(sql::<Bool>(NOT_TERMINAL_STATUS_SQL))
            .filter(diesel::dsl::not(dsl::lifecycle_suspended))
            .filter(diesel::dsl::not(tenant_inactive))
            .filter(diesel::dsl::not(diesel::dsl::exists(held_tenant)))
            .order_by(dsl::current_period_end.asc())
            .limit(limit)
            .for_no_key_update()
            .skip_locked();

        crate::debug_sql(&select_query);

        let claimed: Vec<(SubscriptionId, common_domain::ids::CustomerId, TenantId)> = select_query
            .get_results(conn)
            .await
            .attach("Error while selecting due subscriptions")
            .into_db_result()?;

        if claimed.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<SubscriptionId> = claimed.iter().map(|(id, _, _)| *id).collect();

        // Step 2: Claim them by setting processing_started_at
        let update_query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq_any(&ids))
            .set(dsl::processing_started_at.eq(Some(now)));

        crate::debug_sql(&update_query);

        update_query
            .execute(conn)
            .await
            .attach("Error while claiming subscriptions")
            .into_db_result()?;

        Ok(claimed)
    }

    /// Fetches and locks a subscription for processing, re-validating that it's still due.
    ///
    /// Returns None if the subscription no longer meets the criteria (e.g., was modified
    /// between claim and processing).
    pub async fn get_and_lock_for_processing(
        conn: &mut PgConn,
        lock: &CustomerLock,
        id: SubscriptionId,
    ) -> DbResult<Option<SubscriptionRow>> {
        use crate::schema::subscription::dsl;

        let now = Utc::now().naive_utc();

        let active_statuses = SubscriptionStatusEnum::not_terminal();

        // Re-validate conditions and lock.
        let query = dsl::subscription
            .filter(dsl::id.eq(id))
            .filter(dsl::current_period_end.le(now.date()))
            .filter(dsl::next_retry.is_null().or(dsl::next_retry.le(now)))
            .filter(dsl::status.eq_any(active_statuses))
            .for_no_key_update()
            .skip_locked();

        crate::debug_sql(&query);

        let row: Option<SubscriptionRow> = query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking subscription for processing")
            .into_db_result()?;
        if let Some(row) = &row {
            lock.covers(conn, row.tenant_id, row.customer_id)?;
        }
        Ok(row)
    }

    /// Locks a subscription whose period ends by `as_of` and that no worker has claimed; unlike
    /// the worker's claim it ignores the retry backoff.
    pub async fn lock_due_by(
        conn: &mut PgConn,
        lock: &CustomerLock,
        tenant_id: TenantId,
        id: SubscriptionId,
        as_of: NaiveDate,
    ) -> DbResult<Option<SubscriptionRow>> {
        use crate::schema::subscription::dsl;

        let claim_timeout = Utc::now().naive_utc() - Duration::minutes(CLAIM_TIMEOUT_MINUTES);
        let query = dsl::subscription
            .filter(dsl::id.eq(id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::current_period_end.le(as_of))
            .filter(dsl::status.eq_any(SubscriptionStatusEnum::not_terminal()))
            .filter(
                dsl::processing_started_at
                    .is_null()
                    .or(dsl::processing_started_at.lt(claim_timeout)),
            )
            .for_no_key_update();

        crate::debug_sql(&query);

        let row: Option<SubscriptionRow> = query
            .first(conn)
            .await
            .optional()
            .attach("Error while locking a due subscription")
            .into_db_result()?;
        if let Some(row) = &row {
            lock.covers(conn, row.tenant_id, row.customer_id)?;
        }
        Ok(row)
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

    /// Releases a claim on a subscription (sets processing_started_at to null).
    /// Used when processing fails and we want to allow immediate retry by another worker.
    pub async fn release_claim(conn: &mut PgConn, id: SubscriptionId) -> DbResult<()> {
        use crate::schema::subscription::dsl;

        let query = diesel::update(dsl::subscription)
            .filter(dsl::id.eq(id))
            .set(dsl::processing_started_at.eq(None::<NaiveDateTime>));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while releasing subscription claim")
            .into_db_result()?;

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
impl SubscriptionCycleErrorRowPatch {
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
            .attach("Error while updating subscription errors")
            .into_db_result()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screaming_snake(variant: &SubscriptionStatusEnum) -> String {
        let mut out = String::new();
        for (i, c) in format!("{variant:?}").chars().enumerate() {
            if c.is_uppercase() && i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_uppercase());
        }
        out
    }

    #[test]
    fn not_terminal_status_sql_matches_enum() {
        let values = SubscriptionStatusEnum::not_terminal()
            .iter()
            .map(|s| format!("'{}'", screaming_snake(s)))
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(
            NOT_TERMINAL_STATUS_SQL,
            format!("subscription.status IN ({values})")
        );
    }
}
