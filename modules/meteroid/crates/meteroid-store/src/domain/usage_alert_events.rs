//! Outbox payloads of entitlement and budget alerts and of dated notices. Ids are derived from the
//! crossing they report, so a producer replayed after a failover never notifies twice.
use crate::domain::entitlement_fold::EvalLevel;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{
    BudgetId, CustomerId, EventId, FeatureId, ResourceId, SubscriptionId, TenantId,
};
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

/// The level of a limit an alert is for: the customer's, or one resource's (ADR 0011 §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Customer,
    ResourceType,
}

impl From<EvalLevel> for AlertLevel {
    fn from(level: EvalLevel) -> Self {
        match level {
            EvalLevel::Customer => AlertLevel::Customer,
            EvalLevel::Type(_) => AlertLevel::ResourceType,
        }
    }
}

/// The resource of a resource-type level; `id` is absent for one the customer has not stored.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertResource {
    pub id: Option<ResourceId>,
    pub type_code: String,
    pub key: String,
}

/// Where an entitlement alert fired, in the v2 model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertTarget {
    pub feature_code: String,
    pub level: AlertLevel,
    pub resource: Option<AlertResource>,
    /// `""` for the customer, else `<type code>=<key value>`.
    pub path: String,
}

/// `""` on the wire for none, which the release before reads as a code.
mod empty_as_none {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(code: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(code.as_deref().unwrap_or_default())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(Option::<String>::deserialize(d)?.filter(|code| !code.is_empty()))
    }
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
    /// The target's fields are absent from events written before entitlements v2.
    #[serde(default)]
    pub feature_code: Option<String>,
    /// None for a customer without an active subscription.
    pub subscription_id: Option<SubscriptionId>,
    /// None for a limit counted by the app.
    #[serialize_always]
    #[serde(default, with = "empty_as_none")]
    pub metric_code: Option<String>,
    #[serde(default)]
    pub level: Option<AlertLevel>,
    #[serde(default)]
    pub resource: Option<AlertResource>,
    #[serde(default)]
    pub path: Option<String>,
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
            feature_code: None,
            subscription_id,
            metric_code: reading.metric_code.filter(|code| !code.is_empty()),
            level: None,
            resource: None,
            path: None,
            group: reading.group,
            threshold_pct: reading.threshold_pct,
            used: reading.used,
            limit: reading.limit,
            window_start: reading.window_start,
            window_end: reading.window_end,
            as_of: reading.as_of,
        }
    }

    pub fn with_target(self, target: AlertTarget) -> Self {
        Self {
            feature_code: Some(target.feature_code),
            level: Some(target.level),
            resource: target.resource,
            path: Some(target.path),
            ..self
        }
    }
}

/// `entitlement.too_many_groups`, deprecated: not emitted since entitlements v2, whose checks do
/// not refuse a group a limit stopped tracking. Kept to deliver the events already queued.
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

/// A customer or resource override with an end (ADR 0011 §3.2) ends soon
/// (`entitlement.override_ending_soon`) or has ended (`entitlement.override_ended`), once per end of
/// its lineage (R-148).
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrideEndingEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub feature_id: FeatureId,
    pub feature_code: String,
    /// The override's lineage: its id across edits.
    pub override_id: Uuid,
    pub resource_id: Option<ResourceId>,
    /// The override's value, as entitlements serve it.
    pub value: serde_json::Value,
    pub effect: String,
    pub valid_until: DateTime<Utc>,
    pub reason: Option<String>,
}

impl OverrideEndingEvent {
    pub const ENDING_SOON_ROUTING_KEY: &'static str = "entitlement.override_ending_soon";
    pub const ENDED_ROUTING_KEY: &'static str = "entitlement.override_ended";

    /// One id per lineage, end and kind: a replayed pass publishes the same event.
    pub fn event_id(lineage_id: Uuid, valid_until: DateTime<Utc>, ended: bool) -> EventId {
        let kind = if ended { "ended" } else { "ending" };
        let identity = format!("override|{kind}|{lineage_id}|{}", valid_until.timestamp());
        Uuid::new_v5(&EVENT_NAMESPACE, identity.as_bytes()).into()
    }
}

/// A subscription's trial ends soon (`subscription.trial_ending_soon`), once per trial end (R-158).
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialEndingEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    /// The trial's end, exclusive: billing starts on it.
    pub trial_end: chrono::NaiveDate,
    pub free: bool,
    pub trial_offer_id: Option<Uuid>,
    /// When billing starts, the trial's end converting; none when its end cancels it or waits
    /// for a decision. Unset in events queued before it was published.
    #[serde(default)]
    pub billing_starts_on: Option<chrono::NaiveDate>,
    /// The subscription resolves a payment method now (R-179).
    #[serde(default)]
    pub has_payment_method: Option<bool>,
    /// CONVERT, CANCEL, FALLBACK or MANUAL, as the trial records it.
    #[serde(default)]
    pub end_action: Option<String>,
    /// AWAIT_PAYMENT, CANCEL or SWITCH: what a converting end does without a payment method.
    #[serde(default)]
    pub without_card: Option<String>,
}

impl TrialEndingEvent {
    pub const ROUTING_KEY: &'static str = "subscription.trial_ending_soon";

    pub fn event_id(subscription_id: SubscriptionId, trial_end: chrono::NaiveDate) -> EventId {
        let identity = format!("trial_ending|{subscription_id}|{trial_end}");
        Uuid::new_v5(&EVENT_NAMESPACE, identity.as_bytes()).into()
    }
}

/// A subscription's trial ended (`subscription.trial_ended`), published when its outcome is
/// recorded and again when an awaiting outcome resolves (R-167).
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialEndedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    /// The trial's end, exclusive.
    pub trial_end: chrono::NaiveDate,
    pub outcome: crate::domain::trials::TrialOutcome,
    pub ended_at: NaiveDateTime,
    pub trial_offer_id: Option<Uuid>,
}

impl TrialEndedEvent {
    pub const ROUTING_KEY: &'static str = "subscription.trial_ended";

    pub fn event_id(
        subscription_id: SubscriptionId,
        trial_end: chrono::NaiveDate,
        outcome: crate::domain::trials::TrialOutcome,
    ) -> EventId {
        let identity = format!(
            "trial_ended|{subscription_id}|{trial_end}|{}",
            outcome.as_str()
        );
        Uuid::new_v5(&EVENT_NAMESPACE, identity.as_bytes()).into()
    }
}

/// An offer applied to a subscription (`offer.applied`, ADR 0019): at its creation, from a
/// coupon, or later.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferAppliedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub offer_id: common_domain::ids::OfferId,
    pub application_id: common_domain::ids::OfferApplicationId,
    pub coupon_id: Option<common_domain::ids::CouponId>,
    /// `OfferSource::as_str`.
    pub source: String,
    pub grant_ids: Vec<common_domain::ids::OfferGrantId>,
    pub applied_at: NaiveDateTime,
}

impl OfferAppliedEvent {
    pub const ROUTING_KEY: &'static str = "offer.applied";

    pub fn event_id(application_id: common_domain::ids::OfferApplicationId) -> EventId {
        let identity = format!("offer_applied|{application_id}");
        Uuid::new_v5(&EVENT_NAMESPACE, identity.as_bytes()).into()
    }
}

/// Why a grant ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantEndReason {
    /// Its subscription never started.
    Withdrawn,
    /// Its subscription ended.
    SubscriptionEnded,
    /// Its window ended.
    WindowEnded,
}

/// A grant of an applied offer ended (`grant.ended`): its window over, or before it.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantEndedEvent {
    pub id: EventId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: SubscriptionId,
    pub offer_id: common_domain::ids::OfferId,
    pub application_id: common_domain::ids::OfferApplicationId,
    pub grant_id: common_domain::ids::OfferGrantId,
    /// `GrantKind::as_str`.
    pub kind: String,
    pub reason: GrantEndReason,
    pub ended_at: NaiveDateTime,
}

impl GrantEndedEvent {
    pub const ROUTING_KEY: &'static str = "grant.ended";

    pub fn event_id(
        application_id: common_domain::ids::OfferApplicationId,
        grant_id: common_domain::ids::OfferGrantId,
    ) -> EventId {
        let identity = format!("grant_ended|{application_id}|{grant_id}");
        Uuid::new_v5(&EVENT_NAMESPACE, identity.as_bytes()).into()
    }
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
