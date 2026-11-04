use common_domain::ids::SubscriptionId;

use diesel_models::subscription_adjustments::{
    SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentComponentRowNew,
    SubscriptionAdjustmentRow, SubscriptionAdjustmentRowNew,
};

use crate::domain::plan_minimums::MinimumScope;

/// Materialized minimum-commitment floor on a subscription. Built from either:
/// - a plan version snapshot at subscription creation (`is_override = false`), or
/// - a quote minimum at quote conversion (`is_override = true`), or
/// - an operator override (`is_override = true`).
///
/// Plan changes refresh the snapshot for non-override entries; overrides survive plan changes.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionMinimum {
    pub subscription_id: SubscriptionId,
    pub amount_subunit: i64,
    pub scope: MinimumScope,
    pub is_override: bool,
}

impl SubscriptionMinimum {
    pub fn from_rows(
        row: SubscriptionAdjustmentRow,
        components: Vec<SubscriptionAdjustmentComponentRow>,
    ) -> Self {
        let scope = if row.includes_all_components {
            MinimumScope::AllComponents
        } else {
            MinimumScope::Products(components.into_iter().map(|c| c.product_id).collect())
        };
        Self {
            subscription_id: row.subscription_id,
            amount_subunit: row.amount_subunit,
            scope,
            is_override: row.is_override,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubscriptionMinimumNew {
    pub subscription_id: SubscriptionId,
    pub amount_subunit: i64,
    pub scope: MinimumScope,
    pub is_override: bool,
}

impl SubscriptionMinimumNew {
    pub fn to_row(&self) -> SubscriptionAdjustmentRowNew {
        SubscriptionAdjustmentRowNew {
            subscription_id: self.subscription_id,
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            amount_subunit: self.amount_subunit,
            includes_all_components: matches!(self.scope, MinimumScope::AllComponents),
            is_override: self.is_override,
        }
    }

    pub fn component_rows(&self) -> Vec<SubscriptionAdjustmentComponentRowNew> {
        match &self.scope {
            MinimumScope::AllComponents => vec![],
            MinimumScope::Products(ids) => ids
                .iter()
                .copied()
                .map(|product_id| SubscriptionAdjustmentComponentRowNew {
                    subscription_id: self.subscription_id,
                    kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                    product_id,
                })
                .collect(),
        }
    }

    pub fn from_plan_snapshot(
        plan_minimum: &crate::domain::plan_minimums::PlanMinimum,
        subscription_id: SubscriptionId,
    ) -> Self {
        Self {
            subscription_id,
            amount_subunit: plan_minimum.amount_subunit,
            scope: plan_minimum.scope.clone(),
            is_override: false,
        }
    }
}
