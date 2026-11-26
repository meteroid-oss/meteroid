//! A limit's policy, held on the feature (ADR 0011 §3.1); only a Set to row may override it.
use crate::domain::entitlements::{OverageBehavior, PeriodUnit, ResetPeriod};
use crate::domain::usage_limits::{MAX_GRACE_PCT, validate_thresholds};
use crate::errors::{StoreError, StoreErrorReport};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Resets {
    BillingCycle,
    Calendar,
    Fixed,
    Sliding,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AtLimit {
    Deny,
    AllowAndBill,
    /// Limits counted by the app: what exists stays, nothing new is created.
    DenyNew,
}

/// The length of a calendar, fixed or sliding window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Period {
    pub unit: PeriodUnit,
    pub interval: u32,
}

/// The JSON of `feature.policy` and `entitlement.policy_override`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeaturePolicy {
    pub resets: Resets,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    pub at_limit: AtLimit,
    #[serde(default)]
    pub grace_pct: u32,
    #[serde(default)]
    pub thresholds: Vec<u32>,
}

impl Default for FeaturePolicy {
    fn default() -> Self {
        Self {
            resets: Resets::BillingCycle,
            period: None,
            at_limit: AtLimit::Deny,
            grace_pct: 0,
            thresholds: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("thresholds must be at most 10 ascending percentages between 1 and 100")]
    Thresholds,
    #[error("grace_pct must be at most {MAX_GRACE_PCT}")]
    Grace,
    #[error(
        "a period (unit, interval > 0) is required by CALENDAR, FIXED and SLIDING resets, and refused by the others"
    )]
    Period,
    #[error("a limit counted by the app never resets and is DENY_NEW at its limit")]
    AppCounted,
    #[error("DENY_NEW is for limits counted by the app")]
    DenyNewOnEvents,
    #[error("grace_pct applies to DENY only, and a sliding window takes no thresholds")]
    Inert,
}

impl From<PolicyError> for StoreErrorReport {
    fn from(error: PolicyError) -> Self {
        StoreError::InvalidArgument(error.to_string()).into()
    }
}

/// The per-row limit terms v1 took before policies; `None` keeps the policy's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LegacyLimitTerms {
    pub reset_period: Option<ResetPeriod>,
    pub allow_overage: Option<bool>,
    pub grace_pct: Option<u32>,
    pub thresholds: Option<Vec<u32>>,
}

impl FeaturePolicy {
    /// The policy with `terms` applied, as the entitlements_v2 migration converts a row's terms.
    /// An omitted term that no longer applies (grace past ALLOW, thresholds on a sliding window)
    /// falls back to its default, as v1 did.
    pub fn with_legacy_terms(&self, terms: &LegacyLimitTerms) -> FeaturePolicy {
        let mut policy = self.clone();
        if let Some(reset_period) = &terms.reset_period {
            (policy.resets, policy.period) = match *reset_period {
                ResetPeriod::BillingCycle => (Resets::BillingCycle, None),
                ResetPeriod::Calendar { unit, interval } => {
                    (Resets::Calendar, Some(Period { unit, interval }))
                }
                ResetPeriod::FixedWindow { unit, interval } => {
                    (Resets::Fixed, Some(Period { unit, interval }))
                }
                ResetPeriod::SlidingWindow { unit, interval } => {
                    (Resets::Sliding, Some(Period { unit, interval }))
                }
                ResetPeriod::Never => (Resets::Never, None),
            };
        }
        if let Some(allow) = terms.allow_overage {
            policy.at_limit = match (allow, policy.at_limit) {
                (true, _) => AtLimit::AllowAndBill,
                (false, AtLimit::DenyNew) => AtLimit::DenyNew,
                (false, _) => AtLimit::Deny,
            };
        }
        match terms.grace_pct {
            Some(grace_pct) => policy.grace_pct = grace_pct,
            None if policy.at_limit != AtLimit::Deny => policy.grace_pct = 0,
            None => {}
        }
        match &terms.thresholds {
            Some(thresholds) => policy.thresholds = thresholds.clone(),
            None if policy.resets == Resets::Sliding => policy.thresholds.clear(),
            None => {}
        }
        policy
    }

    pub fn validate(&self, counted_by_app: bool) -> Result<(), PolicyError> {
        validate_thresholds(&self.thresholds).map_err(|_| PolicyError::Thresholds)?;
        if self.grace_pct > MAX_GRACE_PCT {
            return Err(PolicyError::Grace);
        }
        let windowed = matches!(
            self.resets,
            Resets::Calendar | Resets::Fixed | Resets::Sliding
        );
        if windowed != self.period.is_some_and(|p| p.interval > 0) {
            return Err(PolicyError::Period);
        }
        match (counted_by_app, self.at_limit) {
            (true, AtLimit::DenyNew) if self.resets == Resets::Never => {}
            (true, _) => return Err(PolicyError::AppCounted),
            (false, AtLimit::DenyNew) => return Err(PolicyError::DenyNewOnEvents),
            (false, _) => {}
        }
        let sliding_thresholds = self.resets == Resets::Sliding && !self.thresholds.is_empty();
        if (self.grace_pct > 0 && self.at_limit != AtLimit::Deny) || sliding_thresholds {
            return Err(PolicyError::Inert);
        }
        Ok(())
    }

    /// The window the monitors count in. Without a deciding subscription a billing cycle is the calendar month (§3.7).
    pub fn reset_period(&self, subscribed: bool) -> ResetPeriod {
        match (self.resets, self.period) {
            (Resets::BillingCycle, _) if subscribed => ResetPeriod::BillingCycle,
            (Resets::BillingCycle, _) => ResetPeriod::Calendar {
                unit: PeriodUnit::Month,
                interval: 1,
            },
            (Resets::Calendar, Some(Period { unit, interval })) => {
                ResetPeriod::Calendar { unit, interval }
            }
            (Resets::Fixed, Some(Period { unit, interval })) => {
                ResetPeriod::FixedWindow { unit, interval }
            }
            (Resets::Sliding, Some(Period { unit, interval })) => {
                ResetPeriod::SlidingWindow { unit, interval }
            }
            (Resets::Calendar | Resets::Fixed | Resets::Sliding | Resets::Never, _) => {
                ResetPeriod::Never
            }
        }
    }

    pub fn overage_behavior(&self) -> OverageBehavior {
        match self.at_limit {
            AtLimit::AllowAndBill => OverageBehavior::Allow,
            AtLimit::Deny => OverageBehavior::Block {
                grace_period_pct: (self.grace_pct > 0).then_some(self.grace_pct),
            },
            AtLimit::DenyNew => OverageBehavior::Block {
                grace_period_pct: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn every(unit: PeriodUnit, interval: u32) -> Option<Period> {
        Some(Period { unit, interval })
    }

    fn policy(resets: Resets, period: Option<Period>, at_limit: AtLimit) -> FeaturePolicy {
        FeaturePolicy {
            resets,
            period,
            at_limit,
            ..FeaturePolicy::default()
        }
    }

    #[test]
    fn the_stored_json_round_trips() {
        let stored = json!({"resets": "SLIDING", "period": {"unit": "hour", "interval": 6}, "at_limit": "DENY", "grace_pct": 10, "thresholds": [80, 100]});
        let parsed: FeaturePolicy = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(parsed.resets, Resets::Sliding);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), stored);

        let minimal: FeaturePolicy =
            serde_json::from_value(json!({"resets": "NEVER", "at_limit": "DENY_NEW"})).unwrap();
        assert_eq!(minimal, policy(Resets::Never, None, AtLimit::DenyNew));
    }

    #[test]
    fn validation() {
        let with = |f: fn(&mut FeaturePolicy)| {
            let mut p = FeaturePolicy::default();
            f(&mut p);
            p
        };
        let cases: Vec<(FeaturePolicy, bool, Result<(), PolicyError>)> = vec![
            (FeaturePolicy::default(), false, Ok(())),
            (
                policy(Resets::Fixed, every(PeriodUnit::Day, 30), AtLimit::Deny),
                false,
                Ok(()),
            ),
            (
                policy(Resets::Calendar, every(PeriodUnit::Year, 1), AtLimit::Deny),
                false,
                Ok(()),
            ),
            (
                policy(Resets::Fixed, None, AtLimit::Deny),
                false,
                Err(PolicyError::Period),
            ),
            (
                policy(Resets::Calendar, every(PeriodUnit::Week, 0), AtLimit::Deny),
                false,
                Err(PolicyError::Period),
            ),
            (
                policy(
                    Resets::BillingCycle,
                    every(PeriodUnit::Day, 3),
                    AtLimit::Deny,
                ),
                false,
                Err(PolicyError::Period),
            ),
            (policy(Resets::Never, None, AtLimit::DenyNew), true, Ok(())),
            (
                policy(Resets::BillingCycle, None, AtLimit::DenyNew),
                true,
                Err(PolicyError::AppCounted),
            ),
            (
                policy(Resets::Never, None, AtLimit::Deny),
                true,
                Err(PolicyError::AppCounted),
            ),
            (
                policy(Resets::Never, None, AtLimit::DenyNew),
                false,
                Err(PolicyError::DenyNewOnEvents),
            ),
            (
                with(|p| p.thresholds = vec![100, 80]),
                false,
                Err(PolicyError::Thresholds),
            ),
            (
                with(|p| p.grace_pct = MAX_GRACE_PCT + 1),
                false,
                Err(PolicyError::Grace),
            ),
            (
                with(|p| {
                    p.grace_pct = 10;
                    p.at_limit = AtLimit::AllowAndBill
                }),
                false,
                Err(PolicyError::Inert),
            ),
            (
                FeaturePolicy {
                    thresholds: vec![80],
                    ..policy(Resets::Sliding, every(PeriodUnit::Day, 1), AtLimit::Deny)
                },
                false,
                Err(PolicyError::Inert),
            ),
        ];
        for (p, app, expected) in cases {
            assert_eq!(p.validate(app), expected, "{p:?} app={app}");
        }
    }

    #[test]
    fn without_a_subscription_a_billing_cycle_is_the_calendar_month() {
        let month = ResetPeriod::Calendar {
            unit: PeriodUnit::Month,
            interval: 1,
        };
        let cycle = FeaturePolicy::default();
        assert_eq!(cycle.reset_period(true), ResetPeriod::BillingCycle);
        assert_eq!(cycle.reset_period(false), month);
        assert_eq!(
            policy(Resets::Sliding, every(PeriodUnit::Hour, 7), AtLimit::Deny).reset_period(true),
            ResetPeriod::SlidingWindow {
                unit: PeriodUnit::Hour,
                interval: 7
            }
        );
        assert_eq!(
            policy(Resets::Calendar, every(PeriodUnit::Week, 2), AtLimit::Deny).reset_period(false),
            ResetPeriod::Calendar {
                unit: PeriodUnit::Week,
                interval: 2
            }
        );
    }

    #[test]
    fn legacy_terms_apply_as_the_migration_converts_them() {
        let base = FeaturePolicy {
            grace_pct: 10,
            thresholds: vec![80],
            ..FeaturePolicy::default()
        };
        assert_eq!(base.with_legacy_terms(&LegacyLimitTerms::default()), base);

        let allowed = base.with_legacy_terms(&LegacyLimitTerms {
            reset_period: Some(ResetPeriod::Calendar {
                unit: PeriodUnit::Month,
                interval: 1,
            }),
            allow_overage: Some(true),
            ..LegacyLimitTerms::default()
        });
        assert_eq!(
            allowed,
            FeaturePolicy {
                resets: Resets::Calendar,
                period: every(PeriodUnit::Month, 1),
                at_limit: AtLimit::AllowAndBill,
                grace_pct: 0,
                thresholds: vec![80],
            }
        );

        let sliding = base.with_legacy_terms(&LegacyLimitTerms {
            reset_period: Some(ResetPeriod::SlidingWindow {
                unit: PeriodUnit::Day,
                interval: 7,
            }),
            ..LegacyLimitTerms::default()
        });
        assert!(sliding.thresholds.is_empty());
        assert_eq!(sliding.grace_pct, 10);

        let app =
            policy(Resets::Never, None, AtLimit::DenyNew).with_legacy_terms(&LegacyLimitTerms {
                allow_overage: Some(false),
                ..LegacyLimitTerms::default()
            });
        assert_eq!(app.at_limit, AtLimit::DenyNew);
    }

    #[test]
    fn the_overage_behavior_follows_at_limit() {
        let mut p = FeaturePolicy {
            grace_pct: 10,
            ..FeaturePolicy::default()
        };
        assert_eq!(
            p.overage_behavior(),
            OverageBehavior::Block {
                grace_period_pct: Some(10)
            }
        );
        p.at_limit = AtLimit::AllowAndBill;
        assert_eq!(p.overage_behavior(), OverageBehavior::Allow);
    }
}
