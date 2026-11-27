pub(in crate::services) mod activate;
pub(in crate::services) mod amendment;
pub(crate) mod cadence;
pub(in crate::services) mod cancel;
pub(in crate::services) mod coupon_apply;
pub(crate) mod currency;
pub(crate) mod effective_plan;
pub(in crate::services) mod end_date;
mod minimum;
mod mrr;

pub(in crate::services) mod insert;
mod late_pack;
pub(in crate::services) mod levels;
pub mod payment_resolution;
pub(crate) mod phases;
pub(in crate::services) mod plan_change;
pub(in crate::services) mod plan_change_credit;
pub mod proration;
pub(crate) mod slots;
pub(crate) mod start;
pub(in crate::services) mod terminate;
pub(crate) mod terms;
pub(crate) mod timeline_batch;
pub(crate) mod timeline_items;
pub(crate) mod timeline_rewrite;
pub mod utils;

pub use activate::PaymentActivationParams;

/// Where the subscription's fees start billing (R-084): its billing start, or the end of its free
/// trial as its fact records it (a plan change ending it earlier moved it).
pub(crate) async fn billing_from_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<chrono::NaiveDate> {
    let billing_start = subscription
        .billing_start_date
        .unwrap_or(subscription.current_period_start);
    Ok(
        match crate::services::trials::trial_of_tx(conn, subscription.id).await? {
            Some(trial) if trial.free => trial.until.max(billing_start),
            _ => billing_start,
        },
    )
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

/// Whether the subscription's trial is free as its trial fact records it at its start (R-100).
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

/// Whether the subscription's trial is free, as its trial fact records it (I29).
pub(crate) async fn trial_is_free_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<bool> {
    Ok(recorded_trial_free_tx(conn, subscription.id)
        .await?
        .unwrap_or(false))
}
