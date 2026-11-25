pub(in crate::services) mod activate;
pub(in crate::services) mod amendment;
pub(crate) mod cadence;
pub(in crate::services) mod cancel;
pub(crate) mod currency;
mod effective_plan;
pub(in crate::services) mod end_date;
mod minimum;
mod mrr;

pub(in crate::services) mod insert;
mod late_pack;
pub(in crate::services) mod levels;
pub mod payment_resolution;
pub(in crate::services) mod plan_change;
pub(in crate::services) mod plan_change_credit;
pub mod proration;
pub(crate) mod slots;
pub(crate) mod start;
pub(in crate::services) mod terminate;
pub mod utils;

pub use activate::PaymentActivationParams;

/// The plan version the subscription's trial was on: the one it started on, which its first plan
/// switch left (a plan change ends a trial), else its own.
pub(crate) async fn trial_plan_version_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<common_domain::ids::PlanVersionId> {
    Ok(
        diesel_models::subscription_events::SubscriptionEventRow::starting_plan_version(
            conn,
            subscription.id,
        )
        .await?
        .unwrap_or(subscription.plan_version_id),
    )
}

/// Where the subscription's fees start billing (R-084): its billing start, or the end of its free
/// trial, which a plan change ending it earlier moves to that change; as its trial config reads it.
pub(crate) async fn billing_from_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<chrono::NaiveDate> {
    let billing_start = subscription
        .billing_start_date
        .unwrap_or(subscription.current_period_start);
    let Some(days) = subscription.trial_duration.filter(|days| *days > 0) else {
        return Ok(billing_start);
    };
    if !trial_is_free_tx(conn, subscription).await? {
        return Ok(billing_start);
    }
    let first_switch = diesel_models::subscription_events::SubscriptionEventRow::first_plan_switch(
        conn,
        subscription.id,
    )
    .await?;
    Ok(billing_start
        .checked_add_signed(chrono::Duration::days(i64::from(days)))
        .map(|end| first_switch.map_or(end, |(_, switched_on)| end.min(switched_on)))
        .map_or(billing_start, |end| end.max(billing_start)))
}

/// The first paid period's start of a subscription billing from `billing_from` (a free trial's
/// end, else its billing start), in `current_period_start` now: a coupon lasting months redeemed
/// now counts them from it, so one redeemed once billing counts from the period it is in (R-089).
pub(crate) fn coupon_months_from(
    billing_from: chrono::NaiveDate,
    current_period_start: chrono::NaiveDate,
) -> chrono::NaiveDate {
    billing_from.max(current_period_start)
}

/// [`coupon_months_from`] of a stored subscription: what a redemption on it records, and the
/// checkout preview prices with.
pub(crate) async fn coupon_months_from_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<chrono::NaiveDate> {
    let billing_from = billing_from_tx(conn, subscription).await?;
    Ok(coupon_months_from(
        billing_from,
        subscription.current_period_start,
    ))
}

/// [`coupon_months_from_tx`] of the subscription `subscription_id`.
pub(crate) async fn coupon_months_from_id_tx(
    conn: &mut crate::store::PgConn,
    tenant_id: common_domain::ids::TenantId,
    subscription_id: common_domain::ids::SubscriptionId,
) -> crate::StoreResult<chrono::NaiveDate> {
    let subscription: crate::domain::Subscription =
        diesel_models::subscriptions::SubscriptionRow::get_subscription_by_id(
            conn,
            &tenant_id,
            subscription_id,
        )
        .await?
        .try_into()?;
    coupon_months_from_tx(conn, &subscription).await
}

/// Whether the subscription's trial is free as its trial fact records it, from the plan version
/// in force at its start: a later edit of the version never reaches it (R-100).
pub(crate) async fn recorded_trial_free_tx(
    conn: &mut crate::store::PgConn,
    subscription_id: common_domain::ids::SubscriptionId,
) -> crate::StoreResult<Option<bool>> {
    Ok(
        diesel_models::subscription_facts::SubscriptionTrialRow::find(conn, subscription_id)
            .await?
            .map(|trial| trial.free),
    )
}

/// Whether the subscription's current period is its free trial: the invoice ending it opens the
/// first billed period, so the trial's usage is never billed.
pub(crate) async fn in_free_trial_period_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<bool> {
    Ok(
        diesel_models::subscription_facts::SubscriptionTrialRow::find(conn, subscription.id)
            .await?
            .is_some_and(|trial| {
                trial.free && subscription.current_period_end == Some(trial.until_date)
            }),
    )
}

/// Whether the subscription's trial is free: its trial fact (R-100); with none, as activation
/// decided it, by the plan version it started on, free when the version is gone.
pub(crate) async fn trial_is_free_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<bool> {
    if let Some(free) = recorded_trial_free_tx(conn, subscription.id).await? {
        return Ok(free);
    }
    let version = trial_plan_version_tx(conn, subscription).await?;
    Ok(
        diesel_models::plans::PlanRow::get_with_version(conn, version, subscription.tenant_id)
            .await?
            .version
            .is_none_or(|version| version.trial_is_free),
    )
}
