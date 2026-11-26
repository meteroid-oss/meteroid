//! Combining a feature's levels into one decision (ADR 0011 §3.3): every constrained level must allow.
use crate::domain::entitlement_fold::{EvalLevel, GrantValue, LimitValue, ValueOrder};
use crate::domain::entitlement_policy::{AtLimit, FeaturePolicy};
use crate::services::entitlements::limits::rules::hard_ceiling;
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::ResourceTypeId;
use rust_decimal::Decimal;
use std::cmp::Ordering;

/// Within this share of a hard ceiling, a level is near its limit.
const NEAR_SHARE: Decimal = Decimal::from_parts(5, 0, 0, false, 2);

/// When a check refused for usage it could not read is worth retrying.
pub const USAGE_UNAVAILABLE_RETRY: Duration = Duration::seconds(30);

/// Ordered by precedence: the first one a level gives is the answer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, strum::IntoStaticStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckReason {
    ResourceRequired,
    /// The metric a limit counts cannot be loaded: no usage until the feature is fixed.
    MetricNotFound,
    NotSubscribed,
    NotEntitled,
    /// Metering did not answer for the usage a limit is measured against: the check fails
    /// closed, and is worth retrying shortly.
    UsageUnavailable,
    LimitReached,
    BudgetExceeded,
    CreditExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, strum::IntoStaticStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ReasonType {
    Denied,
    /// An integration error rather than a real denial.
    Misconfigured,
    /// Usage could not be read: no decision about the customer, retried shortly.
    Unavailable,
}

impl CheckReason {
    pub fn reason_type(self) -> ReasonType {
        match self {
            CheckReason::ResourceRequired | CheckReason::MetricNotFound => {
                ReasonType::Misconfigured
            }
            CheckReason::UsageUnavailable => ReasonType::Unavailable,
            _ => ReasonType::Denied,
        }
    }
}

/// Why a level's usage cannot be counted as it is set up: retrying does not help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uncountable {
    /// The feature's metric cannot be loaded.
    MetricNotFound,
    /// A type level read without a resource of its type.
    ResourceRequired,
}

impl Uncountable {
    pub fn reason(self) -> CheckReason {
        match self {
            Uncountable::MetricNotFound => CheckReason::MetricNotFound,
            Uncountable::ResourceRequired => CheckReason::ResourceRequired,
        }
    }
}

/// A level's usage as a check, a snapshot or a readout reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelUsage {
    /// `None`: the level counts nothing.
    Read(Option<Decimal>),
    /// Metering failed or did not answer in time.
    Unavailable,
    Uncountable(Uncountable),
}

impl LevelUsage {
    pub fn used(self) -> Option<Decimal> {
        match self {
            LevelUsage::Read(used) => used,
            LevelUsage::Unavailable | LevelUsage::Uncountable(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum LevelValue {
    Value(GrantValue),
    /// No value from the rows: not entitled on the feature's own level, nothing imposed elsewhere.
    Empty,
    /// The check names no resource of the level's type (§3.3, §3.7).
    ResourceRequired,
    /// The check's resource has no subscription including the feature (§3.7).
    NotSubscribed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelOutcome {
    pub level: EvalLevel,
    /// The feature's own level.
    pub own: bool,
    pub value: LevelValue,
    pub policy: FeaturePolicy,
    /// The level's usage, for limits.
    pub usage: LevelUsage,
    /// When the level's window resets; `None` when it never does or slides.
    pub reset_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    pub denial: Option<CheckReason>,
    pub near_limit: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub allowed: bool,
    pub reason: Option<CheckReason>,
    pub reason_type: Option<ReasonType>,
    pub denied_by: Option<EvalLevel>,
    pub near_limit: bool,
    /// The latest time a denial may lift: a limit's reset, or shortly for usage not read; `None`
    /// when a denial does not lift by itself.
    pub retry_after: Option<DateTime<Utc>>,
}

impl LevelOutcome {
    /// Whether this level allows `requested` more units.
    pub fn assess(&self, requested: Decimal) -> Assessment {
        let allowed = Assessment {
            denial: None,
            near_limit: false,
        };
        let denied = |reason| Assessment {
            denial: Some(reason),
            ..allowed
        };
        let limit = match &self.value {
            LevelValue::ResourceRequired => return denied(CheckReason::ResourceRequired),
            LevelValue::NotSubscribed => return denied(CheckReason::NotSubscribed),
            LevelValue::Empty if self.own => return denied(CheckReason::NotEntitled),
            LevelValue::Value(GrantValue::Boolean(false) | GrantValue::Limit(LimitValue::Off)) => {
                return denied(CheckReason::NotEntitled);
            }
            LevelValue::Value(GrantValue::Limit(LimitValue::Amount(limit))) => *limit,
            _ => return allowed,
        };
        let ceiling = match self.policy.at_limit {
            AtLimit::AllowAndBill => return allowed,
            AtLimit::Deny => hard_ceiling(limit, Some(self.policy.grace_pct)),
            AtLimit::DenyNew => limit,
        };
        let used = match self.usage {
            LevelUsage::Read(Some(used)) => used,
            LevelUsage::Uncountable(cause) => return denied(cause.reason()),
            LevelUsage::Read(None) | LevelUsage::Unavailable => {
                return denied(CheckReason::UsageUnavailable);
            }
        };
        let fits = used
            .checked_add(requested)
            .is_some_and(|total| total <= ceiling);
        let margin = limit.checked_mul(NEAR_SHARE).unwrap_or(Decimal::MAX);
        Assessment {
            denial: (!fits).then_some(CheckReason::LimitReached),
            near_limit: ceiling.checked_sub(used).is_none_or(|room| room <= margin),
        }
    }
}

pub fn decide(levels: &[LevelOutcome], requested: Decimal, now: DateTime<Utc>) -> Decision {
    let assessed: Vec<(&LevelOutcome, Assessment)> =
        levels.iter().map(|l| (l, l.assess(requested))).collect();
    let denial = assessed
        .iter()
        .filter_map(|(l, a)| Some((a.denial?, l.level)))
        .min_by_key(|(reason, _)| *reason);
    let retry_after = assessed
        .iter()
        .filter(|(_, a)| a.denial.is_some())
        .map(|(l, a)| match a.denial {
            Some(CheckReason::LimitReached) => l.reset_at,
            Some(CheckReason::UsageUnavailable) => Some(now + USAGE_UNAVAILABLE_RETRY),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .and_then(|resets| resets.into_iter().max());
    Decision {
        allowed: denial.is_none(),
        reason: denial.map(|(reason, _)| reason),
        reason_type: denial.map(|(reason, _)| reason.reason_type()),
        denied_by: denial.map(|(_, level)| level),
        near_limit: assessed.iter().any(|(_, a)| a.near_limit),
        retry_after,
    }
}

/// The own level's value capped by the other levels, in the feature's order.
pub fn combined_value(order: &ValueOrder, levels: &[LevelOutcome]) -> Option<GrantValue> {
    let value = |l: &LevelOutcome| match &l.value {
        LevelValue::Value(v) => Some(v.clone()),
        _ => None,
    };
    let own = levels.iter().find(|l| l.own).and_then(value)?;
    Some(
        levels
            .iter()
            .filter(|l| !l.own)
            .filter_map(value)
            .fold(own, |acc, v| match order.compare(&v, &acc) {
                Some(Ordering::Less) => v,
                _ => acc,
            }),
    )
}

/// The levels a check evaluates: the feature's own first, then the path's others (§3.3).
pub fn levels_on_path(
    feature_level: Option<ResourceTypeId>,
    path_type: Option<ResourceTypeId>,
) -> Vec<EvalLevel> {
    let own = EvalLevel::of_feature(feature_level);
    let path = [Some(EvalLevel::Customer), path_type.map(EvalLevel::Type)];
    std::iter::once(own)
        .chain(path.into_iter().flatten().filter(|l| *l != own))
        .collect()
}

/// A feature whose own level is a type, checked without a resource of it, while a row at that level could reach
/// the customer (§3.3).
pub fn resource_required(
    own: EvalLevel,
    path_type: Option<ResourceTypeId>,
    own_level_has_rows: bool,
) -> bool {
    own_level_has_rows && own.type_id().is_some_and(|t| path_type != Some(t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use common_domain::ids::BaseId;
    use rust_decimal_macros::dec;

    fn n(v: Decimal) -> LevelValue {
        LevelValue::Value(GrantValue::Limit(LimitValue::Amount(v)))
    }

    fn level(level: EvalLevel, own: bool, value: LevelValue) -> LevelOutcome {
        LevelOutcome {
            level,
            own,
            value,
            policy: FeaturePolicy::default(),
            usage: LevelUsage::Unavailable,
            reset_at: None,
        }
    }

    fn limit(level: EvalLevel, own: bool, limit: Decimal, used: Decimal) -> LevelOutcome {
        LevelOutcome {
            usage: LevelUsage::Read(Some(used)),
            ..self::level(level, own, n(limit))
        }
    }

    fn at(day: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + Duration::days(day)
    }

    fn with(mut l: LevelOutcome, at_limit: AtLimit, grace_pct: u32) -> LevelOutcome {
        l.policy.at_limit = at_limit;
        l.policy.grace_pct = grace_pct;
        l
    }

    #[test]
    fn a_limit_whose_usage_could_not_be_read_denies() {
        let unread = level(EvalLevel::Customer, true, n(dec!(10)));
        assert_eq!(
            unread.assess(dec!(1)).denial,
            Some(CheckReason::UsageUnavailable)
        );
        let new_only = with(unread.clone(), AtLimit::DenyNew, 0);
        assert_eq!(
            new_only.assess(dec!(1)).denial,
            Some(CheckReason::UsageUnavailable)
        );
        let billed = with(unread, AtLimit::AllowAndBill, 0);
        assert_eq!(billed.assess(dec!(1)).denial, None);
    }

    #[test]
    fn a_level_assessed_alone() {
        let site = EvalLevel::Type(ResourceTypeId::new());
        let c = EvalLevel::Customer;
        let bool_on = LevelValue::Value(GrantValue::Boolean(true));
        let bool_off = LevelValue::Value(GrantValue::Boolean(false));
        let unlimited = LevelValue::Value(GrantValue::Limit(LimitValue::Unlimited));
        let off = LevelValue::Value(GrantValue::Limit(LimitValue::Off));
        let text = LevelValue::Value(GrantValue::Text("gold".into()));
        let cases = [
            (
                level(site, true, LevelValue::Empty),
                dec!(1),
                Some(CheckReason::NotEntitled),
                false,
            ),
            (level(c, false, LevelValue::Empty), dec!(1), None, false),
            (level(site, true, bool_on), dec!(1), None, false),
            (
                level(c, false, bool_off),
                dec!(1),
                Some(CheckReason::NotEntitled),
                false,
            ),
            (
                level(site, true, off),
                dec!(1),
                Some(CheckReason::NotEntitled),
                false,
            ),
            (level(site, true, unlimited), dec!(1_000_000), None, false),
            (level(site, true, text), dec!(1), None, false),
            (
                level(c, false, LevelValue::ResourceRequired),
                dec!(1),
                Some(CheckReason::ResourceRequired),
                false,
            ),
            (
                level(site, true, LevelValue::NotSubscribed),
                dec!(1),
                Some(CheckReason::NotSubscribed),
                false,
            ),
            (limit(site, true, dec!(100), dec!(50)), dec!(1), None, false),
            (limit(site, true, dec!(100), dec!(96)), dec!(1), None, true),
            (
                limit(site, true, dec!(100), dec!(99)),
                dec!(2),
                Some(CheckReason::LimitReached),
                true,
            ),
            (limit(site, true, dec!(100), dec!(100)), dec!(0), None, true),
            (
                with(limit(site, true, dec!(100), dec!(105)), AtLimit::Deny, 10),
                dec!(5),
                None,
                true,
            ),
            (
                with(limit(site, true, dec!(100), dec!(105)), AtLimit::Deny, 10),
                dec!(6),
                Some(CheckReason::LimitReached),
                true,
            ),
            (
                with(
                    limit(site, true, dec!(100), dec!(500)),
                    AtLimit::AllowAndBill,
                    0,
                ),
                dec!(1),
                None,
                false,
            ),
            (
                with(limit(site, true, dec!(25), dec!(24)), AtLimit::DenyNew, 0),
                dec!(1),
                None,
                true,
            ),
            (
                with(limit(site, true, dec!(25), dec!(25)), AtLimit::DenyNew, 50),
                dec!(1),
                Some(CheckReason::LimitReached),
                true,
            ),
            (
                level(site, true, n(dec!(10))),
                dec!(10),
                Some(CheckReason::UsageUnavailable),
                false,
            ),
        ];
        for (outcome, requested, denial, near_limit) in cases {
            assert_eq!(
                outcome.assess(requested),
                Assessment { denial, near_limit },
                "{outcome:?} requested {requested}"
            );
        }
    }

    #[test]
    fn every_constrained_level_must_allow() {
        let site = EvalLevel::Type(ResourceTypeId::new());
        let customer = EvalLevel::Customer;
        let on = LevelValue::Value(GrantValue::Boolean(true));
        let off = LevelValue::Value(GrantValue::Boolean(false));
        type Case = (Vec<LevelOutcome>, Option<(CheckReason, EvalLevel)>);
        let cases: Vec<Case> = vec![
            (
                vec![level(site, true, on.clone()), level(customer, false, off)],
                Some((CheckReason::NotEntitled, customer)),
            ),
            (
                vec![
                    level(site, true, on.clone()),
                    level(customer, false, LevelValue::Empty),
                ],
                None,
            ),
            (
                vec![
                    level(site, true, LevelValue::Empty),
                    level(customer, false, on),
                ],
                Some((CheckReason::NotEntitled, site)),
            ),
            // Pagecraft: s1 below its 5, Acme at 50 across all sites.
            (
                vec![
                    limit(site, true, dec!(5), dec!(3)),
                    limit(customer, false, dec!(50), dec!(50)),
                ],
                Some((CheckReason::LimitReached, customer)),
            ),
            (
                vec![
                    limit(site, true, dec!(5), dec!(3)),
                    limit(customer, false, dec!(50), dec!(40)),
                ],
                None,
            ),
            (
                vec![
                    limit(site, true, dec!(5), dec!(5)),
                    level(customer, false, LevelValue::ResourceRequired),
                ],
                Some((CheckReason::ResourceRequired, customer)),
            ),
        ];
        for (levels, expected) in cases {
            let d = decide(&levels, dec!(1), at(0));
            assert_eq!(d.allowed, expected.is_none(), "{levels:?}");
            assert_eq!(d.reason.zip(d.denied_by), expected, "{levels:?}");
            assert_eq!(d.reason_type, d.reason.map(CheckReason::reason_type));
        }
    }

    #[test]
    fn near_limit_and_retry_after_across_levels() {
        let site = EvalLevel::Type(ResourceTypeId::new());
        let customer = EvalLevel::Customer;
        let resetting = |l: LevelOutcome, day| LevelOutcome {
            reset_at: Some(at(day)),
            ..l
        };

        let near = decide(
            &[
                limit(site, true, dec!(100), dec!(10)),
                limit(customer, false, dec!(100), dec!(97)),
            ],
            dec!(1),
            at(0),
        );
        assert!(near.allowed && near.near_limit);
        assert_eq!(near.retry_after, None);

        let both = [
            resetting(limit(site, true, dec!(5), dec!(5)), 3),
            resetting(limit(customer, false, dec!(50), dec!(50)), 10),
        ];
        assert_eq!(decide(&both, dec!(1), at(0)).retry_after, Some(at(10)));

        let never = [
            resetting(limit(site, true, dec!(5), dec!(5)), 3),
            limit(customer, false, dec!(50), dec!(50)),
        ];
        assert_eq!(decide(&never, dec!(1), at(0)).retry_after, None);

        let one_denies = [
            resetting(limit(site, true, dec!(5), dec!(5)), 3),
            resetting(limit(customer, false, dec!(50), dec!(1)), 10),
        ];
        assert_eq!(decide(&one_denies, dec!(1), at(0)).retry_after, Some(at(3)));

        let not_entitled = [
            resetting(limit(site, true, dec!(5), dec!(5)), 3),
            level(
                customer,
                false,
                LevelValue::Value(GrantValue::Limit(LimitValue::Off)),
            ),
        ];
        assert_eq!(decide(&not_entitled, dec!(1), at(0)).retry_after, None);
    }

    #[test]
    fn reasons_and_their_types() {
        for reason in [CheckReason::ResourceRequired, CheckReason::MetricNotFound] {
            assert_eq!(reason.reason_type(), ReasonType::Misconfigured);
        }
        for reason in [
            CheckReason::NotSubscribed,
            CheckReason::NotEntitled,
            CheckReason::LimitReached,
            CheckReason::BudgetExceeded,
            CheckReason::CreditExhausted,
        ] {
            assert_eq!(reason.reason_type(), ReasonType::Denied);
        }
        assert_eq!(
            <&str>::from(CheckReason::ResourceRequired),
            "RESOURCE_REQUIRED"
        );
        assert_eq!(<&str>::from(ReasonType::Misconfigured), "misconfigured");
        assert_eq!(
            CheckReason::UsageUnavailable.reason_type(),
            ReasonType::Unavailable
        );
        assert_eq!(<&str>::from(ReasonType::Unavailable), "unavailable");
    }

    #[test]
    fn usage_not_read_is_retried_shortly_unless_another_denial_lasts_longer() {
        let unread = level(EvalLevel::Customer, true, n(dec!(10)));
        let alone = decide(std::slice::from_ref(&unread), dec!(1), at(0));
        assert_eq!(alone.reason, Some(CheckReason::UsageUnavailable));
        assert_eq!(alone.reason_type, Some(ReasonType::Unavailable));
        assert_eq!(alone.retry_after, Some(at(0) + USAGE_UNAVAILABLE_RETRY));

        let site = EvalLevel::Type(ResourceTypeId::new());
        let full = LevelOutcome {
            reset_at: Some(at(3)),
            ..limit(site, false, dec!(5), dec!(5))
        };
        let both = decide(&[unread.clone(), full], dec!(1), at(0));
        assert_eq!(both.reason, Some(CheckReason::UsageUnavailable));
        assert_eq!(both.retry_after, Some(at(3)));

        let off = level(site, false, LevelValue::Value(GrantValue::Boolean(false)));
        assert_eq!(decide(&[unread, off], dec!(1), at(0)).retry_after, None);
    }

    #[test]
    fn usage_that_cannot_be_counted_is_misconfigured_and_never_retried() {
        let site = EvalLevel::Type(ResourceTypeId::new());
        let uncountable = |cause| LevelOutcome {
            usage: LevelUsage::Uncountable(cause),
            ..level(site, true, n(dec!(10)))
        };
        for (cause, reason) in [
            (Uncountable::MetricNotFound, CheckReason::MetricNotFound),
            (Uncountable::ResourceRequired, CheckReason::ResourceRequired),
        ] {
            let d = decide(&[uncountable(cause)], dec!(1), at(0));
            assert!(!d.allowed);
            assert_eq!(d.reason, Some(reason));
            assert_eq!(d.reason_type, Some(ReasonType::Misconfigured));
            assert_eq!(d.retry_after, None, "{cause:?}");
        }
        let billed = with(
            uncountable(Uncountable::MetricNotFound),
            AtLimit::AllowAndBill,
            0,
        );
        assert_eq!(billed.assess(dec!(1)).denial, None);

        let unread = level(EvalLevel::Customer, false, n(dec!(10)));
        let both = decide(
            &[uncountable(Uncountable::MetricNotFound), unread],
            dec!(1),
            at(0),
        );
        assert_eq!(both.reason, Some(CheckReason::MetricNotFound));
        assert_eq!(both.retry_after, None);
    }

    #[test]
    fn an_ordered_value_is_capped_by_the_other_levels() {
        let site = EvalLevel::Type(ResourceTypeId::new());
        let number = |v| LevelValue::Value(GrantValue::Number(v));
        let levels = [
            level(site, true, number(dec!(30))),
            level(EvalLevel::Customer, false, number(dec!(7))),
        ];
        let higher = ValueOrder::Number {
            higher_is_better: true,
        };
        let lower = ValueOrder::Number {
            higher_is_better: false,
        };
        assert_eq!(
            combined_value(&higher, &levels),
            Some(GrantValue::Number(dec!(7)))
        );
        assert_eq!(
            combined_value(&lower, &levels),
            Some(GrantValue::Number(dec!(30)))
        );
        assert_eq!(combined_value(&higher, &levels[1..]), None);
        let text = [level(
            site,
            true,
            LevelValue::Value(GrantValue::Text("a".into())),
        )];
        assert_eq!(
            combined_value(&ValueOrder::Unordered, &text),
            Some(GrantValue::Text("a".into()))
        );
    }

    #[test]
    fn the_path_s_levels_and_a_missing_resource() {
        let (site, tenant) = (ResourceTypeId::new(), ResourceTypeId::new());
        let (s, t, c) = (
            EvalLevel::Type(site),
            EvalLevel::Type(tenant),
            EvalLevel::Customer,
        );
        assert_eq!(levels_on_path(Some(site), Some(site)), vec![s, c]);
        assert_eq!(levels_on_path(Some(site), None), vec![s, c]);
        assert_eq!(levels_on_path(None, Some(site)), vec![c, s]);
        assert_eq!(levels_on_path(None, None), vec![c]);
        assert_eq!(levels_on_path(Some(site), Some(tenant)), vec![s, c, t]);

        let cases = [
            (s, None, true, true),
            (s, Some(tenant), true, true),
            (s, Some(site), true, false),
            // Own level empty, the customer level answers without a resource.
            (s, None, false, false),
            (c, None, true, false),
        ];
        for (own, path, rows, expected) in cases {
            assert_eq!(
                resource_required(own, path, rows),
                expected,
                "{own:?} {path:?} {rows}"
            );
        }
    }
}
