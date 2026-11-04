use crate::domain::entitlements::ConfigValue;
use chrono::{DateTime, Utc};
use common_domain::ids::FeatureId;
use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckReason {
    WithinLimit,
    LimitReached,
    NoEntitlement,
    Disabled,
    BudgetExceeded,
    InsufficientBalance,
    TooManyGroups,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckedFeature {
    pub id: FeatureId,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckWindow {
    pub start: DateTime<Utc>,
    /// `None` for a window that never ends.
    pub end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntitlementCheck {
    pub feature: CheckedFeature,
    pub has_access: bool,
    pub reason: CheckReason,
    pub used: Option<Decimal>,
    pub limit: Option<Decimal>,
    pub remaining: Option<Decimal>,
    pub window: Option<CheckWindow>,
    pub as_of: DateTime<Utc>,
    pub lag_ms: i64,
    pub exact: bool,
    /// A hard limit is close: the answer should not be cached, and checked again after
    /// `retry_after_ms`.
    pub near_limit: bool,
    pub retry_after_ms: Option<i64>,
    /// The entitlement allows overage and the requested units go beyond its limit.
    pub in_overage: bool,
    /// The value of a config feature.
    pub config: Option<ConfigValue>,
}
