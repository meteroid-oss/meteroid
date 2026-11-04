//! Outbox payloads of entitlement and budget alerts. Ids are derived from the
//! crossing they report, so a producer replayed after a failover never notifies twice.
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{BudgetId, CustomerId, EventId, FeatureId, SubscriptionId, TenantId};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use std::collections::BTreeMap;
use uuid::Uuid;

const EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x6d0f_1c2a_7b3e_4f58_9a41_c5e2_0d8b_73a6);

/// The percentage of the limit at which usage counts as the limit reached.
pub const LIMIT_PCT: u32 = 100;

/// What a crossed monitor read when it crossed, whichever it watches.
#[derive(Debug, Clone)]
pub struct AlertReading {
    pub customer_alias: Option<String>,
    pub metric_code: Option<String>,
    pub group: Option<BTreeMap<String, String>>,
    pub threshold_pct: u32,
    pub used: Decimal,
    pub limit: Decimal,
    pub window_start: NaiveDateTime,
    pub window_end: Option<NaiveDateTime>,
    pub as_of: DateTime<Utc>,
}

/// Whether the threshold is the limit itself, which routes to the limit-reached event.
pub const fn is_limit_pct(threshold_pct: u32) -> bool {
    threshold_pct == LIMIT_PCT
}

/// `generation` counts the times the threshold was re-armed in the window: a replay of the same
/// crossing keeps its id, and a later crossing of the same threshold gets a new one.
pub fn alert_event_id(
    monitor_id: Uuid,
    window_key: &str,
    threshold_pct: u32,
    generation: i32,
) -> EventId {
    let identity = format!("{monitor_id}|{window_key}|{threshold_pct}|{generation}");
    Uuid::new_v5(&EVENT_NAMESPACE, identity.as_bytes()).into()
}

/// A feature's usage on a subscription, or of a customer without one, reached one of its
/// thresholds, or its limit (`entitlement.usage_threshold_reached`, `entitlement.limit_reached`).
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitlementAlertEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    pub feature_id: FeatureId,
    /// None for a customer without an active subscription.
    pub subscription_id: Option<SubscriptionId>,
    pub metric_code: String,
    pub group: Option<BTreeMap<String, String>>,
    pub threshold_pct: u32,
    pub used: Decimal,
    pub limit: Decimal,
    pub window_start: NaiveDateTime,
    pub window_end: Option<NaiveDateTime>,
    pub as_of: DateTime<Utc>,
}

impl EntitlementAlertEvent {
    pub const THRESHOLD_ROUTING_KEY: &'static str = "entitlement.usage_threshold_reached";
    pub const LIMIT_ROUTING_KEY: &'static str = "entitlement.limit_reached";

    pub fn new(
        id: EventId,
        tenant_id: TenantId,
        customer_id: CustomerId,
        feature_id: FeatureId,
        subscription_id: Option<SubscriptionId>,
        reading: AlertReading,
    ) -> Self {
        Self {
            id,
            tenant_id,
            customer_id,
            customer_alias: reading.customer_alias,
            feature_id,
            subscription_id,
            metric_code: reading.metric_code.unwrap_or_default(),
            group: reading.group,
            threshold_pct: reading.threshold_pct,
            used: reading.used,
            limit: reading.limit,
            window_start: reading.window_start,
            window_end: reading.window_end,
            as_of: reading.as_of,
        }
    }
}

/// A grouped limit that refuses stopped tracking new groups: its window holds more than the
/// groups kept, so callers of a group not kept are refused (`entitlement.too_many_groups`). Sent
/// once per window.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TooManyGroupsEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    pub feature_id: Option<FeatureId>,
    pub subscription_id: Option<SubscriptionId>,
    pub metric_code: String,
    pub group_by: String,
    pub max_groups: u32,
    pub window_start: NaiveDateTime,
    pub window_end: Option<NaiveDateTime>,
    pub as_of: DateTime<Utc>,
}

impl TooManyGroupsEvent {
    pub const ENTITLEMENT_ROUTING_KEY: &'static str = "entitlement.too_many_groups";
}

/// A customer budget's spend reached one of its thresholds, or the budget
/// (`budget.threshold_reached`, `budget.exceeded`). Amounts are minor units of `currency`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAlertEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    pub budget_id: BudgetId,
    /// Set for a budget that counts one subscription's usage only.
    #[serde(default)]
    pub subscription_id: Option<SubscriptionId>,
    /// An ISO currency, or a credit unit code.
    pub currency: String,
    /// Set for a budget in a credit unit.
    #[serde(default)]
    pub credit_unit_precision: Option<u8>,
    pub threshold_pct: u32,
    pub used_minor: i64,
    pub amount_minor: i64,
    pub window_start: NaiveDateTime,
    pub window_end: Option<NaiveDateTime>,
    pub as_of: DateTime<Utc>,
}

impl BudgetAlertEvent {
    pub const THRESHOLD_ROUTING_KEY: &'static str = "budget.threshold_reached";
    pub const EXCEEDED_ROUTING_KEY: &'static str = "budget.exceeded";

    pub fn new(
        id: EventId,
        tenant_id: TenantId,
        customer_id: CustomerId,
        budget_id: BudgetId,
        currency: String,
        credit_unit_precision: Option<u8>,
        reading: AlertReading,
    ) -> Self {
        Self {
            id,
            tenant_id,
            customer_id,
            customer_alias: reading.customer_alias,
            budget_id,
            subscription_id: None,
            currency,
            credit_unit_precision,
            threshold_pct: reading.threshold_pct,
            used_minor: reading.used.trunc().to_i64().unwrap_or(i64::MAX),
            amount_minor: reading.limit.trunc().to_i64().unwrap_or(i64::MAX),
            window_start: reading.window_start,
            window_end: reading.window_end,
            as_of: reading.as_of,
        }
    }
}
