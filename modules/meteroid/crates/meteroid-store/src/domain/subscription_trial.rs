use common_domain::ids::{PlanId, PlanVersionId};

/// Represents the source of the effective plan for a subscription
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectivePlanSource {
    /// Using the subscription's original plan_version_id
    OriginalPlan,
    /// Using trialing_plan_id during an active trial
    TrialingPlan,
}

/// Information about the effective plan for a subscription based on trial status
#[derive(Debug, Clone)]
pub struct EffectivePlanInfo {
    /// The plan version ID that should be used for this subscription
    pub plan_version_id: PlanVersionId,
    /// The plan ID
    pub plan_id: PlanId,
    /// The plan name
    pub plan_name: String,
    /// Why this plan is the effective plan
    pub source: EffectivePlanSource,
}
