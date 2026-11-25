//! Entitlements in API v2: what a plan version, an add-on or a product grants, each a list of
//! `{feature, value}` written whole with its owner. A product's grants are the defaults of the
//! features scoped to it. The value's shape follows the feature's type; it is read back
//! canonical (sorted by feature code, decimals normalized), so two lists compare.
use crate::AppState;
use crate::v2::problem::{Problem, store_message};
use common_domain::ids::{AliasOr, BaseId, FeatureId, ProductId, TenantId};
use meteroid_store::domain::entitlements::{
    ConfigValue, ConfigValueType, Entitlement, EntitlementSpec, EntitlementValue, Feature,
    FeatureType, OverageBehavior, PeriodUnit, ResetPeriod, requested_thresholds,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::repositories::entitlements::{
    EntitlementsInterface, FeaturePageFilter, validate_value_matches_feature_type,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use utoipa::ToSchema;

/// A feature granted, and how.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// The feature, by code (id or code on input).
    pub feature: String,
    pub value: GrantValue,
}

/// What a grant gives; its type is the feature's.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GrantValue {
    Boolean(BooleanGrant),
    Metered(MeteredGrant),
    Config(ConfigGrant),
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BooleanGrant {
    pub enabled: bool,
}

/// A cap on a metric's usage.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeteredGrant {
    /// None: unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal", example = "10000")]
    pub limit: Option<Decimal>,
    /// When the usage counted against the limit starts again; never by default.
    #[serde(default)]
    pub reset: Reset,
    /// What happens at the limit (a limit only).
    #[serde(default)]
    pub overage: Overage,
    /// `block` only: usage allowed past the limit before it is refused, in percent of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace_pct: Option<u32>,
    /// Percents of the limit (1 to 100) at which an alert is sent, once per period.
    #[serde(default)]
    pub thresholds: Vec<u32>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The event property the limit applies to separately, one limit per value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
}

fn yes() -> bool {
    true
}

/// A static setting; its JSON type follows the feature's `value_type`: a decimal string for
/// `number`, a boolean, a string for `text` and `select`, any JSON for `map` and `json`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfigGrant {
    pub value: serde_json::Value,
}

/// When the usage counted against a limit starts again from zero.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reset {
    BillingCycle(BillingCycleReset),
    Never(NeverReset),
    Calendar(CalendarReset),
    FixedWindow(FixedWindowReset),
    SlidingWindow(SlidingWindowReset),
}

impl Default for Reset {
    /// A lifetime cap.
    fn default() -> Self {
        Reset::Never(NeverReset {})
    }
}

/// At each renewal of the subscription.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BillingCycleReset {}

/// A lifetime cap: the usage never starts again.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NeverReset {}

/// On calendar boundaries (the 1st of each month…), every `interval` calendar `unit`s.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarReset {
    pub unit: Unit,
    /// At least 1.
    #[schema(minimum = 1)]
    pub interval: u32,
}

/// Every `interval` `unit`s from the subscription's activation.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixedWindowReset {
    pub unit: Unit,
    /// At least 1.
    #[schema(minimum = 1)]
    pub interval: u32,
}

/// The usage of the last `interval` `unit`s, always ending now; sends no alerts.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SlidingWindowReset {
    pub unit: Unit,
    /// At least 1.
    #[schema(minimum = 1)]
    pub interval: u32,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Overage {
    /// Usage is refused at the limit, plus `grace_pct`.
    #[default]
    Block,
    /// Usage goes on past the limit, billed by a price on the same metric.
    Allow,
}

// ── Mapping ─────────────────────────────────────────────────────────

fn unit(unit: &PeriodUnit) -> Unit {
    match unit {
        PeriodUnit::Hour => Unit::Hour,
        PeriodUnit::Day => Unit::Day,
        PeriodUnit::Week => Unit::Week,
        PeriodUnit::Month => Unit::Month,
        PeriodUnit::Year => Unit::Year,
    }
}

fn period_unit(unit: Unit) -> PeriodUnit {
    match unit {
        Unit::Hour => PeriodUnit::Hour,
        Unit::Day => PeriodUnit::Day,
        Unit::Week => PeriodUnit::Week,
        Unit::Month => PeriodUnit::Month,
        Unit::Year => PeriodUnit::Year,
    }
}

fn reset(period: &ResetPeriod) -> Reset {
    match period {
        ResetPeriod::BillingCycle => Reset::BillingCycle(BillingCycleReset {}),
        ResetPeriod::Never => Reset::Never(NeverReset {}),
        ResetPeriod::Calendar { unit: u, interval } => Reset::Calendar(CalendarReset {
            unit: unit(u),
            interval: *interval,
        }),
        ResetPeriod::FixedWindow { unit: u, interval } => Reset::FixedWindow(FixedWindowReset {
            unit: unit(u),
            interval: *interval,
        }),
        ResetPeriod::SlidingWindow { unit: u, interval } => {
            Reset::SlidingWindow(SlidingWindowReset {
                unit: unit(u),
                interval: *interval,
            })
        }
    }
}

/// A customer's or a subscription's entitlement reads its reset as a grant writes it.
impl From<ResetPeriod> for Reset {
    fn from(period: ResetPeriod) -> Self {
        reset(&period)
    }
}

fn reset_period(reset: Reset) -> ResetPeriod {
    match reset {
        Reset::BillingCycle(_) => ResetPeriod::BillingCycle,
        Reset::Never(_) => ResetPeriod::Never,
        Reset::Calendar(w) => ResetPeriod::Calendar {
            unit: period_unit(w.unit),
            interval: w.interval,
        },
        Reset::FixedWindow(w) => ResetPeriod::FixedWindow {
            unit: period_unit(w.unit),
            interval: w.interval,
        },
        Reset::SlidingWindow(w) => ResetPeriod::SlidingWindow {
            unit: period_unit(w.unit),
            interval: w.interval,
        },
    }
}

/// A stored value as v2 reads it, canonical.
pub fn grant_value(value: &EntitlementValue) -> GrantValue {
    match value {
        EntitlementValue::Boolean { enabled } => {
            GrantValue::Boolean(BooleanGrant { enabled: *enabled })
        }
        EntitlementValue::Metered {
            limit,
            reset_period,
            overage_behavior,
            thresholds,
            enabled,
            group_by,
        } => {
            let mut thresholds = thresholds.clone();
            thresholds.sort_unstable();
            thresholds.dedup();
            let (overage, grace_pct) = match overage_behavior {
                OverageBehavior::Block { grace_period_pct } => (Overage::Block, *grace_period_pct),
                OverageBehavior::Allow => (Overage::Allow, None),
            };
            GrantValue::Metered(MeteredGrant {
                limit: limit.map(|limit| limit.normalize()),
                reset: reset(reset_period),
                overage,
                grace_pct,
                thresholds,
                enabled: *enabled,
                group_by: group_by.clone(),
            })
        }
        EntitlementValue::Config { value } => GrantValue::Config(ConfigGrant {
            value: match value {
                ConfigValue::Number(number) => {
                    serde_json::Value::String(number.normalize().to_string())
                }
                ConfigValue::Boolean(flag) => serde_json::Value::Bool(*flag),
                ConfigValue::Text(text) => serde_json::Value::String(text.clone()),
                ConfigValue::Json(json) => json.clone(),
            },
        }),
    }
}

fn config_value(
    value: &serde_json::Value,
    value_type: &ConfigValueType,
    path: &str,
) -> Result<ConfigValue, Problem> {
    let refused = |expected: &str| {
        Problem::field(
            format!("{path}/value"),
            "invalid",
            format!("The feature's value is {expected}"),
        )
    };
    match value_type {
        ConfigValueType::Number => {
            let text = match value {
                serde_json::Value::String(text) => text.trim().to_owned(),
                serde_json::Value::Number(number) => number.to_string(),
                _ => return Err(refused("a number, as a decimal string")),
            };
            Decimal::from_str(&text)
                .or_else(|_| Decimal::from_scientific(&text))
                .map(ConfigValue::Number)
                .map_err(|_| refused("a number, as a decimal string"))
        }
        ConfigValueType::Boolean => value
            .as_bool()
            .map(ConfigValue::Boolean)
            .ok_or_else(|| refused("a boolean")),
        ConfigValueType::Text | ConfigValueType::Select { .. } => value
            .as_str()
            .map(|text| ConfigValue::Text(text.to_owned()))
            .ok_or_else(|| refused("a string")),
        ConfigValueType::Map | ConfigValueType::Json => Ok(ConfigValue::Json(value.clone())),
    }
}

/// The stored value of `value` for a feature of `feature_type`, checked as the store checks it;
/// `path` points at the value.
pub fn to_domain(
    value: &GrantValue,
    feature_type: &FeatureType,
    path: &str,
) -> Result<EntitlementValue, Problem> {
    let domain = match (value, feature_type) {
        (GrantValue::Boolean(boolean), FeatureType::Boolean) => EntitlementValue::Boolean {
            enabled: boolean.enabled,
        },
        (GrantValue::Metered(metered), FeatureType::Metered { .. }) => {
            if metered.overage == Overage::Allow && metered.grace_pct.is_some() {
                return Err(Problem::field(
                    format!("{path}/grace_pct"),
                    "invalid",
                    "A grace applies to a limit that blocks only",
                ));
            }
            EntitlementValue::Metered {
                limit: metered.limit,
                reset_period: reset_period(metered.reset),
                overage_behavior: match metered.overage {
                    Overage::Block => OverageBehavior::Block {
                        grace_period_pct: metered.grace_pct,
                    },
                    Overage::Allow => OverageBehavior::Allow,
                },
                thresholds: requested_thresholds(metered.thresholds.clone(), None),
                enabled: metered.enabled,
                group_by: metered
                    .group_by
                    .as_deref()
                    .map(str::trim)
                    .filter(|property| !property.is_empty())
                    .map(str::to_owned),
            }
        }
        (GrantValue::Config(config), FeatureType::Config { value_type }) => {
            EntitlementValue::Config {
                value: config_value(&config.value, value_type, path)?,
            }
        }
        (_, feature_type) => {
            let expected = match feature_type {
                FeatureType::Boolean => "boolean",
                FeatureType::Metered { .. } => "metered",
                FeatureType::Config { .. } => "config",
            };
            return Err(Problem::field(
                format!("{path}/type"),
                "invalid",
                format!("The feature is {expected}: its value is `{expected}`"),
            ));
        }
    };
    validate_value_matches_feature_type(&domain, feature_type)
        .map_err(|error| Problem::field(path, "invalid", store_message(error.current_context())))?;
    Ok(domain)
}

// ── Features ────────────────────────────────────────────────────────

/// A feature path or reference: an id (`feat_…` or a UUID), else a code.
pub fn parse_feature(raw: &str) -> AliasOr<FeatureId> {
    let raw = raw.trim();
    // Responses carry prefixed ids (`feat_…`); a UUID is read too.
    if let Ok(id) = FeatureId::parse_uuid(raw) {
        return AliasOr::Id(id);
    }
    AliasOr::from_str(raw).unwrap_or_else(|_| AliasOr::Alias(raw.to_owned()))
}

/// The tenant's feature of an id or a code (a code names the live feature holding it).
pub async fn feature(state: &AppState, tenant_id: TenantId, raw: &str) -> Result<Feature, Problem> {
    state
        .store
        .get_feature_by_id_or_code(parse_feature(raw), tenant_id)
        .await
        .map_err(|error| {
            let problem = Problem::from(error);
            if problem.status == 404 {
                Problem::not_found(format!("feature {}", raw.trim()))
            } else {
                problem
            }
        })
}

/// The live feature a grant names, refused at `path` when there is none.
async fn granted_feature(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
    path: &str,
) -> Result<Feature, Problem> {
    let feature = feature(state, tenant_id, raw).await.map_err(|problem| {
        if problem.status == 404 {
            Problem::field(
                path,
                "not_found",
                format!("feature {} not found", raw.trim()),
            )
        } else {
            problem
        }
    })?;
    if feature.status == FeatureStatusEnum::Archived {
        return Err(Problem::field(
            path,
            "archived",
            format!("feature {} is archived", feature.code),
        ));
    }
    Ok(feature)
}

async fn resolve(
    state: &AppState,
    tenant_id: TenantId,
    grants: &[Grant],
    path: &str,
) -> Result<Vec<(Feature, EntitlementSpec)>, Problem> {
    crate::v2::bounds::count(
        grants.len(),
        crate::v2::bounds::MAX_ENTITLEMENTS,
        path,
        "entitlements",
    )?;
    let mut seen: HashSet<FeatureId> = HashSet::new();
    let mut resolved = Vec::with_capacity(grants.len());
    for (index, grant) in grants.iter().enumerate() {
        let feature_path = format!("{path}/{index}/feature");
        let feature = granted_feature(state, tenant_id, &grant.feature, &feature_path).await?;
        if !seen.insert(feature.id) {
            return Err(Problem::field(
                feature_path,
                "duplicate",
                format!("feature {} is granted twice", feature.code),
            ));
        }
        let value = to_domain(
            &grant.value,
            &feature.feature_type,
            &format!("{path}/{index}/value"),
        )?;
        resolved.push((
            EntitlementSpec {
                feature_id: feature.id,
                value,
            },
            feature,
        ));
    }
    Ok(resolved
        .into_iter()
        .map(|(spec, feature)| (feature, spec))
        .collect())
}

// ── Helpers for the owners of entitlements ─────────────────────────

/// The grants of `entitlements`, by feature code: those of archived or unknown features left
/// out, sorted by code.
pub async fn render(
    state: &AppState,
    tenant_id: TenantId,
    entitlements: &[Entitlement],
) -> Result<Vec<Grant>, Problem> {
    if entitlements.is_empty() {
        return Ok(Vec::new());
    }
    let codes = live_codes(state, tenant_id, entitlements).await?;
    let mut grants: Vec<Grant> = entitlements
        .iter()
        .filter_map(|entitlement| {
            codes.get(&entitlement.feature_id).map(|code| Grant {
                feature: code.clone(),
                value: grant_value(&entitlement.value),
            })
        })
        .collect();
    grants.sort_by(|a, b| a.feature.cmp(&b.feature));
    Ok(grants)
}

/// [`render`] of several owners' entitlements (a page of add-ons), their features read once.
pub async fn render_each(
    state: &AppState,
    tenant_id: TenantId,
    owners: &[&[Entitlement]],
) -> Result<Vec<Vec<Grant>>, Problem> {
    let all: Vec<Entitlement> = owners
        .iter()
        .flat_map(|entitlements| entitlements.iter().cloned())
        .collect();
    let codes = if all.is_empty() {
        HashMap::new()
    } else {
        live_codes(state, tenant_id, &all).await?
    };
    Ok(owners
        .iter()
        .map(|entitlements| {
            let mut grants: Vec<Grant> = entitlements
                .iter()
                .filter_map(|entitlement| {
                    codes.get(&entitlement.feature_id).map(|code| Grant {
                        feature: code.clone(),
                        value: grant_value(&entitlement.value),
                    })
                })
                .collect();
            grants.sort_by(|a, b| a.feature.cmp(&b.feature));
            grants
        })
        .collect())
}

async fn live_codes(
    state: &AppState,
    tenant_id: TenantId,
    entitlements: &[Entitlement],
) -> Result<HashMap<FeatureId, String>, Problem> {
    let mut ids: Vec<FeatureId> = entitlements.iter().map(|e| e.feature_id).collect();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();
    Ok(state
        .store
        .list_features_by_ids(&ids, tenant_id)
        .await?
        .into_iter()
        .filter(|feature| feature.status != FeatureStatusEnum::Archived)
        .map(|feature| (feature.id, feature.code))
        .collect())
}

/// The grants asked, resolved and checked, refused at `path` (a JSON pointer to the list)
/// before anything is written.
pub async fn specs(
    state: &AppState,
    tenant_id: TenantId,
    grants: &[Grant],
    path: &str,
) -> Result<Vec<EntitlementSpec>, Problem> {
    Ok(resolve(state, tenant_id, grants, path)
        .await?
        .into_iter()
        .map(|(_, spec)| spec)
        .collect())
}

/// The entitlements of `entitlements` whose feature is not live (archived): kept by a caller
/// replacing the visible ones.
pub async fn hidden(
    state: &AppState,
    tenant_id: TenantId,
    entitlements: &[Entitlement],
) -> Result<Vec<EntitlementSpec>, Problem> {
    if entitlements.is_empty() {
        return Ok(Vec::new());
    }
    let codes = live_codes(state, tenant_id, entitlements).await?;
    Ok(entitlements
        .iter()
        .filter(|entitlement| !codes.contains_key(&entitlement.feature_id))
        .map(|entitlement| EntitlementSpec {
            feature_id: entitlement.feature_id,
            value: entitlement.value.clone(),
        })
        .collect())
}

/// What a product grants: the defaults of its live features, by feature code.
pub async fn of_product(
    state: &AppState,
    tenant_id: TenantId,
    product_id: ProductId,
) -> Result<Vec<Grant>, Problem> {
    let features = state
        .store
        .list_features_after(
            tenant_id,
            FeaturePageFilter {
                statuses: Some(vec![FeatureStatusEnum::Active, FeatureStatusEnum::Disabled]),
                product_id: Some(product_id),
            },
            None,
            usize::MAX,
        )
        .await?;
    let mut grants: Vec<Grant> = features
        .into_iter()
        .filter_map(|feature| {
            feature.entitlement.map(|default| Grant {
                feature: feature.code,
                value: grant_value(&default.value),
            })
        })
        .collect();
    grants.sort_by(|a, b| a.feature.cmp(&b.feature));
    Ok(grants)
}

/// The defaults `grants` sets on the product's features, each checked to be one of its own:
/// written by `update_product_with_feature_defaults`, with the product's fields.
pub async fn product_specs(
    state: &AppState,
    tenant_id: TenantId,
    product_id: ProductId,
    grants: &[Grant],
    path: &str,
) -> Result<Vec<EntitlementSpec>, Problem> {
    let resolved = resolve(state, tenant_id, grants, path).await?;
    for (index, (feature, _)) in resolved.iter().enumerate() {
        if feature.product.as_ref().map(|product| product.id) != Some(product_id) {
            return Err(Problem::field(
                format!("{path}/{index}/feature"),
                "invalid",
                format!(
                    "feature {} is not the product's: a product grants its own features",
                    feature.code
                ),
            ));
        }
    }
    Ok(resolved.into_iter().map(|(_, spec)| spec).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_metered_grant_reads_its_defaults() {
        let grant: Grant = serde_json::from_value(serde_json::json!({
            "feature": "api",
            "value": {"type": "metered", "limit": "1000"}
        }))
        .unwrap();
        let GrantValue::Metered(metered) = &grant.value else {
            panic!("metered");
        };
        assert_eq!(metered.reset, Reset::Never(NeverReset {}));
        assert_eq!(metered.overage, Overage::Block);
        assert!(metered.enabled);
        let json = serde_json::to_value(&grant).unwrap();
        assert_eq!(json["value"]["reset"], serde_json::json!({"type": "never"}));
        assert_eq!(json["value"]["overage"], "block");
    }

    #[test]
    fn a_stored_value_reads_back_the_same() {
        let stored = EntitlementValue::Metered {
            limit: Some(Decimal::new(100_000, 2)),
            reset_period: ResetPeriod::Calendar {
                unit: PeriodUnit::Month,
                interval: 1,
            },
            overage_behavior: OverageBehavior::Block {
                grace_period_pct: Some(10),
            },
            thresholds: vec![80, 50],
            enabled: true,
            group_by: None,
        };
        let read = grant_value(&stored);
        let written = to_domain(
            &read,
            &FeatureType::Metered {
                metric_id: common_domain::ids::BillableMetricId::new(),
            },
            "/value",
        )
        .unwrap();
        assert_eq!(grant_value(&written), read);
    }
}
