//! What usage does once the credit it draws on runs out: a plan version term, which each wallet
//! can override for its customer.
use std::collections::BTreeMap;

use common_domain::ids::SubscriptionId;
use diesel_models::enums::{CreditExhaustedRuleEnum, UsageAtZeroEnum};

use crate::errors::StoreError;

/// A plan version's rule, for every denomination (currency or credit unit) its usage is charged
/// in. Subscriptions follow the version they are on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CreditExhaustedRule {
    /// Usage beyond the credit is invoiced.
    #[default]
    Invoice,
    /// The access check refuses usage once the credit the subscription can spend is used up.
    Pause,
    /// Usage beyond the credit is invoiced up to a cap per billing period, then refused. Caps are
    /// in minor units of their denomination, keyed by its upper-case code; usage charged in a
    /// denomination without a cap pauses at zero.
    InvoiceUpTo { caps: BTreeMap<String, i64> },
}

impl CreditExhaustedRule {
    pub fn from_row(rule: CreditExhaustedRuleEnum, caps: &serde_json::Value) -> Self {
        match rule {
            CreditExhaustedRuleEnum::Invoice => Self::Invoice,
            CreditExhaustedRuleEnum::Pause => Self::Pause,
            CreditExhaustedRuleEnum::InvoiceUpTo => Self::InvoiceUpTo {
                caps: serde_json::from_value(caps.clone()).unwrap_or_default(),
            },
        }
    }

    pub fn to_row(&self) -> (CreditExhaustedRuleEnum, serde_json::Value) {
        match self {
            Self::Invoice => (CreditExhaustedRuleEnum::Invoice, serde_json::json!({})),
            Self::Pause => (CreditExhaustedRuleEnum::Pause, serde_json::json!({})),
            Self::InvoiceUpTo { caps } => (
                CreditExhaustedRuleEnum::InvoiceUpTo,
                serde_json::to_value(caps).unwrap_or_else(|_| serde_json::json!({})),
            ),
        }
    }

    /// Caps are positive and keyed by upper-case codes, at least one of them.
    pub fn validated(self) -> Result<Self, StoreError> {
        let Self::InvoiceUpTo { caps } = self else {
            return Ok(self);
        };
        if caps.is_empty() {
            return Err(StoreError::InvalidArgument(
                "invoicing usage up to a cap needs a cap".to_owned(),
            ));
        }
        let mut normalized = BTreeMap::new();
        for (denomination, cap) in caps {
            let code = denomination.trim().to_uppercase();
            if code.is_empty() || cap <= 0 {
                return Err(StoreError::InvalidArgument(
                    "a cap names its currency or credit unit and is positive".to_owned(),
                ));
            }
            normalized.insert(code, cap);
        }
        Ok(Self::InvoiceUpTo { caps: normalized })
    }

    /// What may be invoiced beyond the credit of `denomination` per billing period, in its minor
    /// units: `None` when nothing stops the usage.
    pub fn allowance(&self, denomination: &str) -> Option<i64> {
        match self {
            Self::Invoice => None,
            Self::Pause => Some(0),
            Self::InvoiceUpTo { caps } => Some(
                caps.get(&denomination.trim().to_uppercase())
                    .copied()
                    .unwrap_or(0)
                    .max(0),
            ),
        }
    }
}

/// A wallet's override of the plans' rule, for its customer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageAtZero {
    #[default]
    FollowPlan,
    AlwaysPause,
    NeverPause,
}

impl From<UsageAtZeroEnum> for UsageAtZero {
    fn from(value: UsageAtZeroEnum) -> Self {
        match value {
            UsageAtZeroEnum::FollowPlan => Self::FollowPlan,
            UsageAtZeroEnum::AlwaysPause => Self::AlwaysPause,
            UsageAtZeroEnum::NeverPause => Self::NeverPause,
        }
    }
}

impl From<UsageAtZero> for UsageAtZeroEnum {
    fn from(value: UsageAtZero) -> Self {
        match value {
            UsageAtZero::FollowPlan => Self::FollowPlan,
            UsageAtZero::AlwaysPause => Self::AlwaysPause,
            UsageAtZero::NeverPause => Self::NeverPause,
        }
    }
}

impl UsageAtZero {
    /// What may be invoiced beyond the credit of `denomination` per billing period for usage of a
    /// subscription on `plan` (`None` when its plan is unknown): this override, else the plan's.
    pub fn allowance(self, plan: Option<&CreditExhaustedRule>, denomination: &str) -> Option<i64> {
        match self {
            Self::AlwaysPause => Some(0),
            Self::NeverPause => None,
            Self::FollowPlan => plan.and_then(|rule| rule.allowance(denomination)),
        }
    }
}

/// The plan rule one active subscription of the customer applies to a wallet's denomination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletPlanRule {
    pub subscription_id: SubscriptionId,
    pub plan_name: String,
    pub rule: CreditExhaustedRule,
}

/// The plan rules of `rows` by denomination, then by subscription.
pub fn wallet_plan_rules(
    rows: Vec<diesel_models::customer_credit::PlanCreditRuleRow>,
) -> std::collections::HashMap<String, Vec<WalletPlanRule>> {
    let mut rules: std::collections::HashMap<String, Vec<WalletPlanRule>> =
        std::collections::HashMap::new();
    for row in rows {
        rules
            .entry(row.denomination)
            .or_default()
            .push(WalletPlanRule {
                subscription_id: row.subscription_id,
                plan_name: row.plan_name,
                rule: CreditExhaustedRule::from_row(
                    row.credit_exhausted_rule,
                    &row.credit_exhausted_caps,
                ),
            });
    }
    rules
}

/// Whether usage stops as soon as the wallet's credit is used up, for at least one subscription
/// charging usage in it.
pub fn pauses_at_zero(
    usage_at_zero: UsageAtZero,
    plans: &[WalletPlanRule],
    denomination: &str,
) -> bool {
    match usage_at_zero {
        UsageAtZero::AlwaysPause => true,
        UsageAtZero::NeverPause => false,
        UsageAtZero::FollowPlan => plans
            .iter()
            .any(|plan| plan.rule.allowance(denomination) == Some(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn up_to(caps: &[(&str, i64)]) -> CreditExhaustedRule {
        CreditExhaustedRule::InvoiceUpTo {
            caps: caps
                .iter()
                .map(|(code, cap)| ((*code).to_owned(), *cap))
                .collect(),
        }
    }

    #[test]
    fn the_wallet_override_wins_over_the_plan_unless_it_follows_it() {
        let pause = CreditExhaustedRule::Pause;
        let invoice = CreditExhaustedRule::Invoice;
        assert_eq!(
            UsageAtZero::FollowPlan.allowance(Some(&pause), "EUR"),
            Some(0)
        );
        assert_eq!(
            UsageAtZero::FollowPlan.allowance(Some(&invoice), "EUR"),
            None
        );
        assert_eq!(UsageAtZero::FollowPlan.allowance(None, "EUR"), None);
        assert_eq!(
            UsageAtZero::AlwaysPause.allowance(Some(&invoice), "EUR"),
            Some(0)
        );
        assert_eq!(UsageAtZero::NeverPause.allowance(Some(&pause), "EUR"), None);
        let capped = up_to(&[("EUR", 5_000)]);
        assert_eq!(
            UsageAtZero::NeverPause.allowance(Some(&capped), "EUR"),
            None
        );
        assert_eq!(
            UsageAtZero::AlwaysPause.allowance(Some(&capped), "EUR"),
            Some(0)
        );
    }

    #[test]
    fn a_cap_applies_to_its_own_denomination_and_any_other_pauses_at_zero() {
        let capped = up_to(&[("EUR", 5_000)]);
        assert_eq!(capped.allowance("eur "), Some(5_000));
        assert_eq!(capped.allowance("CREDITS"), Some(0));
        assert_eq!(CreditExhaustedRule::Pause.allowance("EUR"), Some(0));
        assert_eq!(CreditExhaustedRule::Invoice.allowance("EUR"), None);
    }

    #[test]
    fn caps_are_validated_and_normalized() {
        assert_eq!(
            up_to(&[(" eur", 100)]).validated().unwrap(),
            up_to(&[("EUR", 100)])
        );
        assert!(up_to(&[]).validated().is_err());
        assert!(up_to(&[("EUR", 0)]).validated().is_err());
        assert!(up_to(&[("", 10)]).validated().is_err());
        assert_eq!(
            CreditExhaustedRule::Pause.validated().unwrap(),
            CreditExhaustedRule::Pause
        );
    }

    #[test]
    fn a_rule_round_trips_through_its_row() {
        for rule in [
            CreditExhaustedRule::Invoice,
            CreditExhaustedRule::Pause,
            up_to(&[("EUR", 100), ("CREDITS", 2_500)]),
        ] {
            let (kind, caps) = rule.to_row();
            assert_eq!(CreditExhaustedRule::from_row(kind, &caps), rule);
        }
    }

    #[test]
    fn a_wallet_pauses_at_zero_when_one_of_its_plans_does() {
        let plan = |rule| WalletPlanRule {
            subscription_id: SubscriptionId::new(),
            plan_name: "Studio".to_owned(),
            rule,
        };
        let plans = [
            plan(CreditExhaustedRule::Invoice),
            plan(CreditExhaustedRule::Pause),
        ];
        assert!(pauses_at_zero(UsageAtZero::FollowPlan, &plans, "EUR"));
        assert!(!pauses_at_zero(UsageAtZero::NeverPause, &plans, "EUR"));
        assert!(!pauses_at_zero(
            UsageAtZero::FollowPlan,
            &[plan(up_to(&[("EUR", 100)]))],
            "EUR"
        ));
        assert!(pauses_at_zero(UsageAtZero::AlwaysPause, &[], "EUR"));
        assert!(!pauses_at_zero(UsageAtZero::FollowPlan, &[], "EUR"));
    }
}
