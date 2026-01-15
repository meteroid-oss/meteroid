use crate::StoreResult;
use crate::domain::SubscriptionStatusEnum;
use crate::domain::subscription_trial::{EffectivePlanInfo, EffectivePlanSource};
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::TenantId;
use diesel_models::plan_versions::PlanVersionFilter;
use diesel_models::plans::PlanRow;
use diesel_models::subscriptions::SubscriptionRow;

impl Services {
    /// Resolves the effective plan for a subscription based on its trial status.
    ///
    /// The effective plan determines which plan's features/pricing should apply:
    /// - During active trial with `trialing_plan_id` set: use the trialing plan
    /// - Otherwise: use the subscription's original plan
    ///
    /// Note: The subscription always keeps its original `plan_version_id` - this method
    /// just tells you which plan should be "in effect" for the subscription.
    pub async fn resolve_effective_plan(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
    ) -> StoreResult<EffectivePlanInfo> {
        // Fetch the plan with version to get trial config and plan info
        let plan_with_version =
            PlanRow::get_with_version(conn, subscription.plan_version_id, subscription.tenant_id)
                .await?;

        let plan = plan_with_version.plan;
        let plan_version = plan_with_version
            .version
            .expect("Plan version should exist for subscription");

        let status: SubscriptionStatusEnum = subscription.status.clone().into();

        // Only during TrialActive do we potentially use a different plan
        if status == SubscriptionStatusEnum::TrialActive
            && let Some(trialing_plan_id) = plan_version.trialing_plan_id
            && let Ok(trialing_plan_with_version) = PlanRow::get_with_version_by_id(
                conn,
                trialing_plan_id,
                subscription.tenant_id,
                PlanVersionFilter::Active,
            )
            .await
            && let Some(trialing_version) = trialing_plan_with_version.version
        {
            return Ok(EffectivePlanInfo {
                plan_version_id: trialing_version.id,
                plan_id: trialing_plan_id,
                plan_name: trialing_plan_with_version.plan.name,
                source: EffectivePlanSource::TrialingPlan,
            });
        }

        // For all other cases, use the original plan
        Ok(EffectivePlanInfo {
            plan_version_id: subscription.plan_version_id,
            plan_id: plan.id,
            plan_name: plan.name,
            source: EffectivePlanSource::OriginalPlan,
        })
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
