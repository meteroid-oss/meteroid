pub(in crate::services) mod activate;
pub(in crate::services) mod amendment;
pub(crate) mod cadence;
pub(in crate::services) mod cancel;
mod effective_plan;
mod minimum;
mod mrr;

pub(in crate::services) mod fee_terms;
pub(in crate::services) mod insert;
mod late_pack;
pub mod payment_resolution;
pub(in crate::services) mod plan_change;
pub(in crate::services) mod plan_change_credit;
pub mod proration;
pub(crate) mod slots;
pub(crate) mod start;
pub(in crate::services) mod terminate;
pub mod utils;

pub use activate::PaymentActivationParams;

/// Whether the subscription's trial is free, as activation decides it: by its plan version, a
/// trial length set on the subscription alone included; free when the version is gone.
pub(crate) async fn trial_is_free_tx(
    conn: &mut crate::store::PgConn,
    subscription: &crate::domain::Subscription,
) -> crate::StoreResult<bool> {
    Ok(diesel_models::plans::PlanRow::get_with_version(
        conn,
        subscription.plan_version_id,
        subscription.tenant_id,
    )
    .await?
    .version
    .is_none_or(|version| version.trial_is_free))
}
