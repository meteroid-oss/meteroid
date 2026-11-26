//! The entitlement check (ADR-0013 §25), core in every edition (ADR-0011 §3.14): whether the
//! customer may use `requested` more of a feature now, at every level. Core answers from the
//! usage read at check time; the Enterprise edition answers from the usage it keeps live and adds
//! budget and credit denials, through the store's `live_check` hook.
use super::resources::{ResourceName, query_resource, resource_input};
use crate::AppState;
use crate::entitlements::mapping::bracketed_params;
use crate::v2::entitlements::model::{
    Codes, EntitlementEffect, EntitlementOwner, EntitlementValue, LevelKind, LevelResource,
    level_kind, level_resource, owner_of,
};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::ids;
use crate::v2::problem::Problem;
use crate::v2::refs::{FeatureRef, ResourceTypeRef, SubscriptionRef};
use axum::Extension;
use axum::Json;
use axum::extract::{RawQuery, State};
use common_domain::actor::Actor;
use common_domain::ids::{
    ApiTokenId, CustomerId, EntitlementId, FeatureId, SubscriptionId, TenantId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlement_checks::{
    AppCountExplain as DomainAppCount, CheckHint as DomainHint, CheckLevel as DomainLevel,
    CheckLevelExplain, CheckReason as DomainReason, CheckRequest, CheckResponse,
    DecidingCandidate as DomainCandidate, ReasonType, UsageSource as DomainSource,
};
use meteroid_store::domain::entitlement_fold::{
    Contribution as DomainContribution, Discarded, Tier,
};
use meteroid_store::domain::entitlements::Feature;
use meteroid_store::domain::resources::ResourceInput;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use utoipa::{IntoParams, ToSchema};

/// Why the check, or a level, refuses. When several apply, the first in this list is given.
/// New reasons may be added: treat an unknown one as a refusal.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckReason {
    /// The feature applies per resource and the check names none: pass `resource`.
    ResourceRequired,
    /// The metric a limit counts no longer exists: refused until the feature is fixed.
    MetricNotFound,
    /// No subscription of the resource the check names includes the feature.
    NotSubscribed,
    /// Nothing grants the feature at this level, or it is off.
    NotEntitled,
    /// The usage a limit counts could not be read, so the check refuses rather than guess;
    /// retry at `retry_at`.
    UsageUnavailable,
    /// The request would take usage past the limit.
    LimitReached,
    /// A hard budget covering the feature's usage is spent (Enterprise edition).
    BudgetExceeded,
    /// The usage is paid from prepaid credit and none is left (Enterprise edition).
    CreditExhausted,
}

impl From<DomainReason> for CheckReason {
    fn from(reason: DomainReason) -> Self {
        match reason {
            DomainReason::ResourceRequired => CheckReason::ResourceRequired,
            DomainReason::MetricNotFound => CheckReason::MetricNotFound,
            DomainReason::NotSubscribed => CheckReason::NotSubscribed,
            DomainReason::NotEntitled => CheckReason::NotEntitled,
            DomainReason::UsageUnavailable => CheckReason::UsageUnavailable,
            DomainReason::LimitReached => CheckReason::LimitReached,
            DomainReason::BudgetExceeded => CheckReason::BudgetExceeded,
            DomainReason::CreditExhausted => CheckReason::CreditExhausted,
        }
    }
}

/// What kind of refusal it is, to decide what to do about it.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckReasonType {
    /// The customer is not entitled to more: show an upgrade, or stop the use.
    Denied,
    /// The call or the feature is wrong (a resource left out, a metric deleted): fix it;
    /// retrying does not help.
    Misconfigured,
    /// Usage could not be read in time: nothing is decided about the customer. Retry at
    /// `retry_at`, or apply your own fallback.
    Unavailable,
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckHint {
    /// The check named no resource, so the customer's own subscriptions answered, while
    /// subscriptions of its resources include the feature too: pass `resource`.
    ResourceOmitted,
}

/// Where the usage came from: `read` when checking, or `live`: kept up to date as usage events
/// arrive (Enterprise edition).
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Read,
    Live,
}

/// Where a row sits in the order rows apply in, first to last: the feature's defaults, the
/// plan, the plan version, the subscription, its add-ons, the customer's override, and an
/// override for one resource.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntitlementTier {
    Default,
    Plan,
    PlanVersion,
    Subscription,
    AddOn,
    CustomerOverride,
    OverrideForResource,
}

impl From<Tier> for EntitlementTier {
    fn from(tier: Tier) -> Self {
        match tier {
            Tier::Default => EntitlementTier::Default,
            Tier::Plan => EntitlementTier::Plan,
            Tier::PlanVersion => EntitlementTier::PlanVersion,
            Tier::Subscription => EntitlementTier::Subscription,
            Tier::AddOn => EntitlementTier::AddOn,
            Tier::Customer => EntitlementTier::CustomerOverride,
            Tier::ResourceOverride => EntitlementTier::OverrideForResource,
        }
    }
}

/// Why a row changed nothing.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiscardReason {
    /// A `set` row later in the order replaced it.
    ReplacedByHigherTier,
    /// Another `set` row of its tier won: a more specific condition, a better value, or an
    /// older row.
    LostWithinTier,
    /// A `cap_at` over nothing granted.
    NoEffect,
    /// An `add` or `raise_to` on a level where nothing is granted to add to or raise.
    NoBaseOnLevel,
    /// Its owner cannot give this effect to this feature.
    EffectNotAllowed,
    /// The resource does not meet its `condition`.
    ConditionNotMet,
    /// Outside its `valid_from` and `valid_until`.
    NotInEffect,
}

impl From<Discarded> for DiscardReason {
    fn from(discarded: Discarded) -> Self {
        match discarded {
            Discarded::ReplacedByHigherTier => DiscardReason::ReplacedByHigherTier,
            Discarded::LostWithinTier => DiscardReason::LostWithinTier,
            Discarded::NoEffect => DiscardReason::NoEffect,
            Discarded::NoBaseOnLevel => DiscardReason::NoBaseOnLevel,
            Discarded::EffectNotAllowed => DiscardReason::EffectNotAllowed,
            Discarded::ConditionNotMet => DiscardReason::ConditionNotMet,
            Discarded::NotInEffect => DiscardReason::NotInEffect,
        }
    }
}

/// A row's part in a level's value.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Contribution {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub entitlement: EntitlementId,
    pub owner: EntitlementOwner,
    pub tier: EntitlementTier,
    pub effect: EntitlementEffect,
    pub value: EntitlementValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<BTreeMap<String, Vec<String>>>,
    /// An add-on's instances bought; 1 for other rows.
    pub quantity: u32,
    /// A limit per unit of a managed quantity: the units the deciding subscription holds, which
    /// multiply `value` (`running` is the product).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<u32>,
    /// The row gives its limit a policy of its own.
    pub policy_override: bool,
    /// The value once this row applied; absent when it changed nothing (`discarded`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<EntitlementValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discarded: Option<DiscardReason>,
}

impl From<DomainContribution> for Contribution {
    fn from(c: DomainContribution) -> Self {
        Contribution {
            entitlement: c.lineage_id,
            owner: owner_of(&c.source, c.source_name),
            tier: c.tier.into(),
            effect: c.effect.into(),
            value: (&c.value).into(),
            condition: c.condition.map(|condition| condition.0),
            quantity: c.quantity,
            units: c.units,
            policy_override: c.policy_override,
            running: c.running.as_ref().map(Into::into),
            discarded: c.discarded.map(Into::into),
        }
    }
}

/// A subscription including the feature at the level, with the value its rows give there.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct DecidingCandidate {
    pub subscription: SubscriptionRef,
    /// The resource it is for; absent for a subscription of the customer as a whole.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<LevelResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
}

fn candidate(c: DomainCandidate, codes: &Codes) -> DecidingCandidate {
    DecidingCandidate {
        subscription: codes.subscription(c.subscription_id),
        resource: c.resource_path.as_deref().and_then(level_resource),
        value: c.value.as_ref().map(Into::into),
    }
}

/// The last `set` of a count.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct LastSet {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub count: Decimal,
    pub at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// A limit counted by your app: the count, the last `set` and the adds since.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct AppCountExplain {
    /// The resource counted; absent for the customer as a whole.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<LevelResource>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub count: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_set: Option<LastSet>,
    pub adds_since: i64,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub adds_sum: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_add_at: Option<ApiDateTime>,
}

impl From<DomainAppCount> for AppCountExplain {
    fn from(a: DomainAppCount) -> Self {
        AppCountExplain {
            resource: level_resource(&a.path),
            count: a.count.normalize(),
            last_set: match (a.last_set_value, a.last_set_at) {
                (Some(count), Some(at)) => Some(LastSet {
                    count: count.normalize(),
                    at: at.into(),
                    key: a.last_set_key,
                }),
                _ => None,
            },
            adds_since: a.adds_since,
            adds_sum: a.adds_sum.normalize(),
            last_add_at: a.last_add_at.map(Into::into),
        }
    }
}

/// How a level's value was made: each row's part, the subscriptions that could decide it, and
/// the counts your app reported.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct LevelExplain {
    pub contributions: Vec<Contribution>,
    pub candidates: Vec<DecidingCandidate>,
    pub app_counts: Vec<AppCountExplain>,
}

fn explained(e: CheckLevelExplain, codes: &Codes) -> LevelExplain {
    LevelExplain {
        contributions: e.contributions.into_iter().map(Into::into).collect(),
        candidates: e
            .candidates
            .into_iter()
            .map(|c| candidate(c, codes))
            .collect(),
        app_counts: e.app_counts.into_iter().map(Into::into).collect(),
    }
}

/// A level, as checked: the customer as a whole, or the resource the check names.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CheckLevel {
    pub level: LevelKind,
    /// With `level: resource_type`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<ResourceTypeRef>,
    /// With `level: resource_type`: the resource this level is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<LevelResource>,
    /// The level the feature applies per (`applies_per`, else the customer): its value is the
    /// feature's. The other levels can only cap it.
    pub primary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    pub near_limit: bool,
    /// Past the limit of a level that allows usage past it and bills it.
    pub in_overage: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<ApiDateTime>,
    /// The subscription whose value applies here, when several include the feature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_subscription: Option<SubscriptionRef>,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CheckReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<LevelExplain>,
}

/// Whether the customer may use `requested` more of a feature now: every level must allow.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct EntitlementCheck {
    pub feature: FeatureRef,
    pub allowed: bool,
    /// Absent when allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CheckReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_type: Option<CheckReasonType>,
    /// The primary level's value, capped by the other levels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    /// `used`, `limit` and `remaining` are the refusing level's, else the primary level's; absent
    /// without a limit.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    /// Left before the limit, not counting its grace.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub remaining: Option<Decimal>,
    /// A level is within 5% of a limit that refuses, or past it: check before every use, and do
    /// not cache an answer that allows.
    pub near_limit: bool,
    /// The request goes past a limit that allows usage past it and bills it: allowed, and billed.
    pub in_overage: bool,
    /// When a refusal may lift: the latest reset among the refusing levels when each is a limit
    /// that resets, or 30 seconds ahead when usage could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<ApiDateTime>,
    /// The primary level first.
    pub levels: Vec<CheckLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<CheckHint>,
    /// Attributes the request asserted that the stored resource holds other values of: the
    /// stored ones were used.
    pub attributes_ignored: Vec<String>,
    /// `usage_unavailable`: a level's usage could not be read, its `used` is absent.
    pub warnings: Vec<String>,
    pub usage_source: UsageSource,
    /// When the usage the answer uses was counted: now, or, while usage cannot be read, the
    /// last live count (`usage_source: live`).
    pub as_of: ApiDateTime,
}

fn level(l: DomainLevel, primary: bool, codes: &Codes) -> CheckLevel {
    let (kind, resource_type) = level_kind(l.level, codes);
    CheckLevel {
        level: kind,
        resource_type,
        resource: level_resource(&l.path),
        primary,
        value: l.value.as_ref().map(Into::into),
        used: l.used.map(|used| used.normalize()),
        limit: l.limit.map(|limit| limit.normalize()),
        near_limit: l.near_limit,
        in_overage: l.in_overage,
        reset_at: l.reset_at.map(Into::into),
        deciding_subscription: l.deciding_subscription.map(|id| codes.subscription(id)),
        allowed: l.denial.is_none(),
        reason: l.denial.map(Into::into),
        explain: l.explain.map(|e| explained(e, codes)),
    }
}

pub(crate) fn check_of(answer: CheckResponse, codes: &Codes) -> EntitlementCheck {
    let primary = answer
        .levels
        .iter()
        .find(|l| l.denial.is_some())
        .or(answer.levels.first());
    let used = primary.and_then(|l| l.used);
    let limit = primary.and_then(|l| l.limit);
    EntitlementCheck {
        feature: FeatureRef {
            id: answer.feature.id,
            code: answer.feature.code,
        },
        allowed: answer.allowed,
        reason: answer.reason.map(Into::into),
        reason_type: answer.reason_type.map(|t| match t {
            ReasonType::Denied => CheckReasonType::Denied,
            ReasonType::Misconfigured => CheckReasonType::Misconfigured,
            ReasonType::Unavailable => CheckReasonType::Unavailable,
        }),
        value: answer.value.as_ref().map(Into::into),
        remaining: limit.map(|l| {
            (l - used.unwrap_or_default())
                .max(Decimal::ZERO)
                .normalize()
        }),
        used: used.map(|u| u.normalize()),
        limit: limit.map(|l| l.normalize()),
        near_limit: answer.near_limit,
        in_overage: answer.levels.iter().any(|l| l.in_overage),
        retry_at: answer.retry_after.map(Into::into),
        levels: answer
            .levels
            .into_iter()
            .enumerate()
            .map(|(i, l)| level(l, i == 0, codes))
            .collect(),
        hint: answer
            .hint
            .map(|DomainHint::ResourceOmitted| CheckHint::ResourceOmitted),
        attributes_ignored: answer.attributes_ignored,
        warnings: answer.warnings,
        usage_source: match answer.usage_source {
            DomainSource::Read => UsageSource::Read,
            DomainSource::Monitor => UsageSource::Live,
        },
        as_of: answer.usage_as_of.unwrap_or_else(common_clock::now).into(),
    }
}

/// What a check asks. `explain` and `current_count` take a secret key.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CheckEntitlement {
    /// The feature, by code or id.
    pub feature: String,
    /// The resource the use is for, stored or not: `{"id": "res_…"}`, or
    /// `{"type": "site", "key": "s1"}`. Left out: the customer as a whole.
    #[serde(default)]
    pub resource: Option<ResourceName>,
    /// Attributes of the resource, `{"site.environment": "staging"}`, for rows with a
    /// `condition`; a stored resource's own values win.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// The units about to be used; 1 by default.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub requested: Option<Decimal>,
    /// A limit counted by your app: the count to check, instead of the last one reported.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub current_count: Option<Decimal>,
    /// Adds how each level's value was made (`explain` on each level).
    #[serde(default)]
    pub explain: bool,
}

/// The query of the check's `GET`; `resource` (or `resource[type]` and `resource[key]`) and
/// `attributes[<type>.<attribute>]=<value>` are read beside it.
#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct CheckQuery {
    /// The units about to be used, a decimal string; 1 by default.
    pub requested: Option<String>,
    /// A limit counted by your app: the count to check, instead of the last one reported.
    pub current_count: Option<String>,
    /// Adds how each level's value was made (`explain` on each level).
    #[serde(default)]
    pub explain: bool,
}

/// A check as the store answers it: its resource read as a key and its value.
#[derive(Debug, Clone)]
pub(crate) struct CheckAsked {
    pub feature: String,
    pub input: ResourceInput,
    pub requested: Option<Decimal>,
    pub current_count: Option<Decimal>,
    pub explain: bool,
}

/// v1's inputs no v2 check takes: named, rather than answered as if they were not sent.
pub(crate) const DROPPED_INPUTS: &[&str] = &[
    "group",
    "group_by",
    "as_of",
    "subscription",
    "subscription_id",
];

pub(crate) fn decimal(raw: Option<&str>, parameter: &str) -> Result<Option<Decimal>, Problem> {
    raw.map(|raw| {
        Decimal::from_str(raw.trim())
            .map_err(|_| Problem::field(parameter, "invalid", "Not a decimal number"))
    })
    .transpose()
}

/// Refuses the first query parameter among `names` (`group[region]` counts as `group`).
pub(crate) fn refuse_dropped(raw: Option<&str>, names: &[&str]) -> Result<(), Problem> {
    let dropped = url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes())
        .map(|(key, _)| key.split('[').next().unwrap_or_default().to_owned())
        .find(|name| names.contains(&name.as_str()));
    match dropped {
        Some(name) => Err(Problem::field(
            name.as_str(),
            "invalid",
            format!("The check takes no `{name}`"),
        )),
        None => Ok(()),
    }
}

/// The `GET` form's request, as the `POST` body carries it.
fn request_of_query(
    feature: String,
    query: CheckQuery,
    raw: Option<&str>,
) -> Result<CheckEntitlement, Problem> {
    refuse_dropped(raw, DROPPED_INPUTS)?;
    Ok(CheckEntitlement {
        feature,
        resource: query_resource(raw)?,
        attributes: bracketed_params(raw, "attributes"),
        requested: decimal(query.requested.as_deref(), "requested")?,
        current_count: decimal(query.current_count.as_deref(), "current_count")?,
        explain: query.explain,
    })
}

/// The request with its resource resolved; `pointer` prefixes the body's fields (`/`), or is
/// empty for the query's parameters.
async fn asked(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    request: CheckEntitlement,
    pointer: &str,
) -> Result<CheckAsked, Problem> {
    let input = resource_input(
        state,
        tenant_id,
        customer_id,
        request.resource.as_ref(),
        request.attributes,
        &format!("{pointer}resource"),
    )
    .await?;
    Ok(CheckAsked {
        feature: request.feature,
        input,
        requested: request.requested,
        current_count: request.current_count,
        explain: request.explain,
    })
}

/// The check answered for `customer_id`; `pointer` prefixes the fields refused (`/` for a
/// body, empty for a query), the store's refusals of the resource at `resource_field`.
pub(crate) async fn check(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    request: CheckAsked,
    api_key_id: Option<ApiTokenId>,
    pointer: &str,
    resource_field: &str,
) -> Result<EntitlementCheck, Problem> {
    let at = |field: &str| format!("{pointer}{field}");
    if request.requested.is_some_and(|r| r.is_sign_negative()) {
        return Err(Problem::field(
            at("requested"),
            "invalid",
            "requested is not negative",
        ));
    }
    if request.current_count.is_some_and(|c| c.is_sign_negative()) {
        return Err(Problem::field(
            at("current_count"),
            "invalid",
            "current_count is not negative",
        ));
    }
    let feature = ids::id_or_code::<FeatureId>(&request.feature);
    let answer = state
        .store
        .check_entitlement(
            tenant_id,
            customer_id,
            feature,
            CheckRequest {
                resources: request.input.resources,
                attributes: request.input.attributes,
                requested: request.requested,
                current_count: request.current_count,
                explain: request.explain,
                subscription_id: None,
                api_key_id,
            },
        )
        .await
        .map_err(|error| Problem::at(at(resource_field), error))?;
    let deciding: Vec<SubscriptionId> = answer
        .levels
        .iter()
        .flat_map(|l| {
            l.deciding_subscription.into_iter().chain(
                l.explain
                    .iter()
                    .flat_map(|e| e.candidates.iter().map(|c| c.subscription_id)),
            )
        })
        .collect();
    let codes =
        Codes::of_features(state, tenant_id, std::iter::empty::<&Feature>(), deciding).await?;
    Ok(check_of(answer, &codes))
}

/// The key a server call came with, for the store's log of asserted attributes.
fn api_key_of(auth: &AuthorizedAsTenant) -> Option<ApiTokenId> {
    match auth.as_actor() {
        Actor::ApiToken { id } => Some(id),
        _ => None,
    }
}

/// Check an entitlement
///
/// Whether the customer may use `requested` more of a feature now. It is checked at each level
/// (the customer as a whole, and the resource named by `resource`), and every level must allow.
/// A limit that bills usage past it never refuses: `in_overage` says when the request goes past
/// it. When the usage of a limit that refuses cannot be read, the check refuses with
/// `usage_unavailable` (`reason_type: unavailable`) and a `retry_at` 30 seconds ahead. A check
/// changes nothing, so it takes no `Idempotency-Key`.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlements/check",
    operation_id = "check_entitlement",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    request_body(
        content = CheckEntitlement,
        examples(
            ("customer" = (summary = "A limit of the customer", value = json!({"feature": "api_calls", "requested": "1"}))),
            ("resource" = (summary = "A limit per site", value = json!({"feature": "projects", "resource": {"type": "site", "key": "s1"}}))),
        )
    ),
    responses(
        (status = 200, description = "Whether the customer may use the requested units", body = EntitlementCheck, examples(
            ("allowed" = (summary = "Allowed", value = json!(examples::allowed()))),
            ("limit_reached" = (summary = "Refused by the site's limit", value = json!(examples::limit_reached()))),
        )),
        (status = 404, description = "The customer or the feature is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A negative `requested` or `current_count`, a resource not the customer's or of no type, or an unknown attribute or value", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The check could not be answered in time; retry", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn check_entitlement(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Body(body): Body<CheckEntitlement>,
) -> Result<Json<EntitlementCheck>, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let request = asked(&state, auth.tenant_id, customer.id, body, "/").await?;
    let answer = check(
        &state,
        auth.tenant_id,
        customer.id,
        request,
        api_key_of(&auth),
        "/",
        "resource",
    )
    .await?;
    Ok(Json(answer))
}

/// Check an entitlement (GET)
///
/// The `GET` form of `POST …/entitlements/check`, its inputs as query parameters: `resource`
/// (an id) or `resource[type]` and `resource[key]`, and `attributes[<type>.<attribute>]=<value>`.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlements/{feature}/check",
    operation_id = "check_entitlement_by_query",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("feature" = String, Path, description = "Feature code or id"),
        CheckQuery,
        ("resource" = Option<String>, Query, description = "A stored resource of the customer, by id"),
        ("resource[type]" = Option<String>, Query, description = "With `resource[key]`: a resource, stored or not, by its type's code"),
        ("resource[key]" = Option<String>, Query, description = "With `resource[type]`: the resource's key value"),
        ("attributes" = Option<BTreeMap<String, String>>, Query, style = DeepObject, explode, description = "Asserted attributes, as `attributes[<type>.<attribute>]=<value>`"),
    ),
    responses(
        (status = 200, description = "Whether the customer may use the requested units", body = EntitlementCheck, examples(
            ("allowed" = (summary = "Allowed: `GET …/entitlements/api_calls/check`", value = json!(examples::allowed()))),
            ("limit_reached" = (summary = "Refused by the site's limit: `GET …/entitlements/projects/check?resource[type]=site&resource[key]=s1`", value = json!(examples::limit_reached()))),
        )),
        (status = 404, description = "The customer or the feature is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The check could not be answered in time; retry", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn check_entitlement_by_query(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, feature)): Path<(String, String)>,
    Query(query): Query<CheckQuery>,
    RawQuery(raw): RawQuery,
) -> Result<Json<EntitlementCheck>, Problem> {
    let request = request_of_query(feature, query, raw.as_deref())?;
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let request = asked(&state, auth.tenant_id, customer.id, request, "").await?;
    let answer = check(
        &state,
        auth.tenant_id,
        customer.id,
        request,
        api_key_of(&auth),
        "",
        "resource",
    )
    .await?;
    Ok(Json(answer))
}

/// The answers the check's documentation shows, for its `POST` and its `GET`.
mod examples {
    use serde_json::{Value, json};

    const SUBSCRIPTION: &str = "sub_3zX5cV7bN9mQ1wE3rT5yU7";
    const AS_OF: &str = "2026-11-26T10:15:00.000Z";

    pub fn allowed() -> Value {
        json!({
            "feature": {"id": "feat_2xK9mQ4rT7vW1yZ3bN5cP8", "code": "api_calls"},
            "allowed": true,
            "value": {"type": "limit", "limit": "10000"},
            "used": "8250",
            "limit": "10000",
            "remaining": "1750",
            "near_limit": false,
            "in_overage": false,
            "levels": [{
                "level": "customer",
                "primary": true,
                "value": {"type": "limit", "limit": "10000"},
                "used": "8250",
                "limit": "10000",
                "near_limit": false,
                "in_overage": false,
                "reset_at": "2026-12-01T00:00:00.000Z",
                "deciding_subscription": {"id": SUBSCRIPTION},
                "allowed": true
            }],
            "attributes_ignored": [],
            "warnings": [],
            "usage_source": "read",
            "as_of": AS_OF
        })
    }

    /// 5 projects per site, at most 50 across all sites: the site's 5 are used.
    pub fn limit_reached() -> Value {
        let site = json!({"id": "rty_5tY7uI9oP1aS3dF5gH7jK9", "code": "site"});
        json!({
            "feature": {"id": "feat_6hJ3kL8nP2qR5sT9vX1zB4", "code": "projects"},
            "allowed": false,
            "reason": "limit_reached",
            "reason_type": "denied",
            "value": {"type": "limit", "limit": "5"},
            "used": "5",
            "limit": "5",
            "remaining": "0",
            "near_limit": true,
            "in_overage": false,
            "levels": [
                {
                    "level": "resource_type",
                    "resource_type": site,
                    "resource": {"type": "site", "key": "s1"},
                    "primary": true,
                    "value": {"type": "limit", "limit": "5"},
                    "used": "5",
                    "limit": "5",
                    "near_limit": true,
                    "in_overage": false,
                    "deciding_subscription": {"id": SUBSCRIPTION},
                    "allowed": false,
                    "reason": "limit_reached"
                },
                {
                    "level": "customer",
                    "primary": false,
                    "value": {"type": "limit", "limit": "50"},
                    "used": "32",
                    "limit": "50",
                    "near_limit": false,
                    "in_overage": false,
                    "deciding_subscription": {"id": SUBSCRIPTION},
                    "allowed": true
                }
            ],
            "attributes_ignored": [],
            "warnings": [],
            "usage_source": "read",
            "as_of": AS_OF
        })
    }
}
