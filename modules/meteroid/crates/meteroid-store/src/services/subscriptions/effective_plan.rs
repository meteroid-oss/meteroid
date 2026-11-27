use crate::StoreResult;
use crate::domain::SubscriptionStatusEnum;
use crate::domain::subscription_trial::{EffectivePlanInfo, EffectivePlanSource};
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::TenantId;
use diesel_models::plans::PlanRow;
use diesel_models::subscriptions::SubscriptionRow;

impl Services {
    /// Resolves the effective plan for a subscription based on its trial status.
    ///
    /// During an active trial giving another plan, that plan's active version; else its own.
    ///
    /// Note: The subscription always keeps its original `plan_version_id` - this method
    /// just tells you which plan should be "in effect" for the subscription.
    pub async fn resolve_effective_plan(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
    ) -> StoreResult<EffectivePlanInfo> {
        effective_plan(conn, subscription).await
    }

    /// Convenience method that fetches the subscription and resolves its effective plan
    pub async fn get_subscription_effective_plan(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
    ) -> StoreResult<EffectivePlanInfo> {
        let subscription_row =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await?;
        self.resolve_effective_plan(conn, &subscription_row.subscription)
            .await
    }
}

/// The plan in effect: during an active trial giving another plan, that plan's active version (its
/// trial fact's trial plan, ADR 0011 §3.9), else the subscription's own.
pub(crate) async fn effective_plan(
    conn: &mut PgConn,
    subscription: &SubscriptionRow,
) -> StoreResult<EffectivePlanInfo> {
    let tiers = effective_plan_versions(
        conn,
        subscription.tenant_id,
        std::slice::from_ref(subscription),
    )
    .await?;
    let version = tiers
        .get(&subscription.id)
        .copied()
        .unwrap_or(subscription.plan_version_id);
    let plan = PlanRow::get_with_version(conn, version, subscription.tenant_id)
        .await?
        .plan;
    Ok(EffectivePlanInfo {
        plan_version_id: version,
        plan_id: plan.id,
        plan_name: plan.name,
        source: if version == subscription.plan_version_id {
            EffectivePlanSource::OriginalPlan
        } else {
            EffectivePlanSource::TrialingPlan
        },
    })
}

/// `effective_plan`'s version for each of `subscriptions`, in one query.
pub(crate) async fn effective_plan_versions(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscriptions: &[SubscriptionRow],
) -> StoreResult<
    std::collections::HashMap<
        common_domain::ids::SubscriptionId,
        common_domain::ids::PlanVersionId,
    >,
> {
    let trialing: Vec<_> = subscriptions
        .iter()
        .filter(|s| {
            SubscriptionStatusEnum::from(s.status.clone()) == SubscriptionStatusEnum::TrialActive
        })
        .map(|s| s.id)
        .collect();
    let tiers =
        diesel_models::query::trial_plans::trial_plan_versions(conn, tenant_id, &trialing).await?;
    Ok(subscriptions
        .iter()
        .map(|s| (s.id, tiers.get(&s.id).copied().unwrap_or(s.plan_version_id)))
        .collect())
}
