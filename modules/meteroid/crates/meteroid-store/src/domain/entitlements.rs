use crate::domain::enums::{EntitlementModeEnum, FeatureStatusEnum};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, BillableMetricId, EntitlementEntityId, EntitlementId, FeatureId, ProductId, TenantId,
};
use diesel_models::entitlements::{EntitlementRow, FeatureRowNew, FeatureWithProductRow};
use diesel_models::enums::{
    ConfigValueTypeEnum as DbConfigValueTypeEnum, FeatureStatusEnum as DbFeatureStatusEnum,
    FeatureTypeEnum as DbFeatureTypeEnum,
};
use rust_decimal::Decimal;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodUnit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// When the consumed counter resets.
///
/// - `BillingCycle`: resets at each invoice period boundary (aligns with the customer's billing date).
/// - `Calendar`: resets at fixed wall-clock boundaries — e.g. every Monday 00:00 UTC regardless of
///   when the subscription started. Predictable for the customer but not tied to billing.
/// - `FixedWindow`: resets every `interval` units measured from first use.
/// - `SlidingWindow`: the window is always the last `interval` units from now.
/// - `Never`: the limit is a lifetime cap; it never resets.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResetPeriod {
    BillingCycle,
    Calendar { unit: PeriodUnit, interval: u32 },
    FixedWindow { unit: PeriodUnit, interval: u32 }, // resets every N units from the first use
    SlidingWindow { unit: PeriodUnit, interval: u32 }, // last N units from now
    Never,
}

/// What happens when a customer exceeds their entitlement limit.
///
/// - `Block`: requests are rejected once the limit (plus optional `grace_period_pct`) is
///   reached. Hard enforcement.
/// - `Allow`: usage continues past the limit without restriction. Overage billing, if any,
///   is handled by a separate price component tied to the same billable metric.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OverageBehavior {
    Block {
        #[serde(skip_serializing_if = "Option::is_none")]
        grace_period_pct: Option<u32>,
    },
    Allow,
}

#[derive(Clone, Debug)]
pub struct EntitlementUsage {
    pub consumed: Option<Decimal>,
    pub remaining: Option<Decimal>,
    pub period_start: Option<DateTime<Utc>>,
    pub reset_at: Option<DateTime<Utc>>,
}

/// A synchronously-resolved configuration value carried by a `Config` feature. Unlike
/// `Metered` it has no usage/counter and never touches the metering pipeline: it is stored in
/// Postgres and resolved through the entitlement chain like a static setting. `Json` is the
/// structured-metadata escape hatch — an object holds several related config fields at once.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ConfigValue {
    Number(Decimal),
    Boolean(bool),
    Text(String),
    Json(serde_json::Value),
}

/// Authoritative value type of a Config feature — declared once at creation and immutable.
/// `Map`/`Json` both store a JSON `ConfigValue` (Map is the key/value-editor variant); `Select`
/// additionally constrains the value to `options`.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValueType {
    Number,
    Boolean,
    Text,
    Map,
    Json,
    Select { options: Vec<String> },
}

impl ConfigValueType {
    /// Split into the persisted (enum discriminator, options) pair.
    pub fn into_db(self) -> (DbConfigValueTypeEnum, Option<Vec<String>>) {
        match self {
            ConfigValueType::Number => (DbConfigValueTypeEnum::Number, None),
            ConfigValueType::Boolean => (DbConfigValueTypeEnum::Boolean, None),
            ConfigValueType::Text => (DbConfigValueTypeEnum::Text, None),
            ConfigValueType::Map => (DbConfigValueTypeEnum::Map, None),
            ConfigValueType::Json => (DbConfigValueTypeEnum::Json, None),
            ConfigValueType::Select { options } => (DbConfigValueTypeEnum::Select, Some(options)),
        }
    }

    /// Rebuild from the persisted discriminator and options column. A missing discriminator
    /// (legacy rows) falls back to `Select` when options exist, else `Text`.
    pub fn from_db(kind: Option<DbConfigValueTypeEnum>, options: Option<Vec<String>>) -> Self {
        match kind {
            Some(DbConfigValueTypeEnum::Number) => ConfigValueType::Number,
            Some(DbConfigValueTypeEnum::Boolean) => ConfigValueType::Boolean,
            Some(DbConfigValueTypeEnum::Text) => ConfigValueType::Text,
            Some(DbConfigValueTypeEnum::Map) => ConfigValueType::Map,
            Some(DbConfigValueTypeEnum::Json) => ConfigValueType::Json,
            Some(DbConfigValueTypeEnum::Select) => ConfigValueType::Select {
                options: options.unwrap_or_default(),
            },
            None => match options {
                Some(options) => ConfigValueType::Select { options },
                None => ConfigValueType::Text,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub enum FeatureType {
    Boolean,
    Metered {
        metric_id: BillableMetricId,
    },
    /// Static, typed configuration value. No metric, no usage — the value is resolved
    /// synchronously from Postgres. `value_type` fixes the shape (number/bool/text/map/json or a
    /// constrained select) for every entitlement of this feature.
    Config {
        value_type: ConfigValueType,
    },
}

impl FeatureType {
    pub fn metered(metric_id: BillableMetricId) -> Self {
        FeatureType::Metered { metric_id }
    }
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub id: FeatureId,
    pub tenant_id: TenantId,
    /// Product this feature belongs to. `None` for tenant-global features.
    pub product: Option<FeatureProductRef>,
    pub name: String,
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    pub code: String,
    pub description: Option<String>,
    pub feature_type: FeatureType,
    pub status: FeatureStatusEnum,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub entitlement: Option<Entitlement>,
}

impl TryFrom<FeatureWithProductRow> for Feature {
    type Error = StoreErrorReport;

    fn try_from(row: FeatureWithProductRow) -> Result<Self, Self::Error> {
        let FeatureWithProductRow { feature, product } = row;
        let feature_type = match feature.feature_type {
            DbFeatureTypeEnum::Boolean => FeatureType::Boolean,
            DbFeatureTypeEnum::Metered => FeatureType::Metered {
                metric_id: feature.metric_id.ok_or_else(|| {
                    error_stack::Report::new(StoreError::InvalidArgument(
                        "metered feature missing metric_id".into(),
                    ))
                })?,
            },
            DbFeatureTypeEnum::Config => FeatureType::Config {
                value_type: ConfigValueType::from_db(
                    feature.config_value_type,
                    feature
                        .config_options
                        .map(|opts| opts.into_iter().flatten().collect()),
                ),
            },
        };
        Ok(Feature {
            id: feature.id,
            tenant_id: feature.tenant_id,
            product: product.map(|p| FeatureProductRef {
                id: p.id,
                name: p.name,
            }),
            name: feature.name,
            code: feature.code,
            description: feature.description,
            feature_type,
            status: feature.status.into(),
            created_at: feature.created_at,
            updated_at: feature.updated_at,
            entitlement: None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct FeatureNew {
    pub tenant_id: TenantId,
    pub product_id: Option<ProductId>,
    pub name: String,
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    pub code: String,
    pub description: Option<String>,
    pub feature_type: FeatureType,
    pub entitlement: Option<EntitlementValue>,
}

impl From<FeatureNew> for FeatureRowNew {
    fn from(f: FeatureNew) -> Self {
        let FeatureNew {
            tenant_id,
            product_id,
            name,
            code,
            description,
            feature_type,
            entitlement: _,
        } = f;
        let (feature_type_enum, metric_id, config_options, config_value_type) = match feature_type {
            FeatureType::Boolean => (DbFeatureTypeEnum::Boolean, None, None, None),
            FeatureType::Metered { metric_id } => {
                (DbFeatureTypeEnum::Metered, Some(metric_id), None, None)
            }
            FeatureType::Config { value_type } => {
                let (kind, options) = value_type.into_db();
                let options = options.map(|opts| opts.into_iter().map(Some).collect());
                (DbFeatureTypeEnum::Config, None, options, Some(kind))
            }
        };
        FeatureRowNew {
            id: FeatureId::new(),
            tenant_id,
            product_id,
            name,
            code,
            description,
            feature_type: feature_type_enum,
            status: DbFeatureStatusEnum::Active,
            metric_id,
            config_options,
            config_value_type,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FeatureUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub product_id: Option<Option<ProductId>>,
    /// `Some` ⇒ replace the select-config feature's allowed options. `None` ⇒ leave unchanged.
    pub config_options: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entitlement {
    pub id: EntitlementId,
    pub tenant_id: TenantId,
    pub feature_id: FeatureId,
    pub entity: EntitlementEntityId,
    /// Server-resolved composition mode. Set automatically when the entitlement is created
    /// from the owning entity (e.g. AddOn with `max_instances_per_subscription > 1` → `Stack`,
    /// everything else → `Override`). Not configurable through the public API.
    pub mode: EntitlementModeEnum,
    pub value: EntitlementValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<EntitlementRow> for Entitlement {
    type Error = StoreErrorReport;

    fn try_from(row: EntitlementRow) -> Result<Self, Self::Error> {
        let entity = EntitlementEntityId::from(&row);
        let mode: EntitlementModeEnum = row.mode.clone().into();
        let value: EntitlementValue = serde_json::from_value(row.value).map_err(|e| {
            error_stack::Report::new(StoreError::InvalidArgument(format!(
                "invalid entitlement value: {e}"
            )))
        })?;
        Ok(Entitlement {
            id: row.id,
            tenant_id: row.tenant_id,
            feature_id: row.feature_id,
            entity,
            mode,
            value,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Caller-provided entitlement spec for inline creation when the entity is implicit
/// (the thing being created supplies the entity; caller supplies which feature and what value).
#[derive(Clone, Debug)]
pub struct EntitlementSpec {
    pub feature_id: FeatureId,
    pub value: EntitlementValue,
}

#[derive(Clone, Debug)]
pub struct EntitlementNew {
    pub tenant_id: TenantId,
    pub feature_id: FeatureId,
    pub entity: EntitlementEntityId,
    pub value: EntitlementValue,
}

#[derive(Clone, Debug)]
pub struct EntitlementUpdate {
    pub value: Option<EntitlementValue>,
}

fn default_reset_period() -> ResetPeriod {
    ResetPeriod::Never
}

fn default_metered_enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    from = "StoredEntitlementValue"
)]
pub enum EntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<Decimal>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        /// Percentages of the limit that send an alert when usage reaches them, ascending.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        thresholds: Vec<u32>,
        enabled: bool,
        /// Event property the limit is tracked per, e.g. one allowance per seat or per repository.
        #[serde(skip_serializing_if = "Option::is_none")]
        group_by: Option<String>,
    },
    Config {
        value: ConfigValue,
    },
}

/// `EntitlementValue` as stored: values saved before `thresholds` carry a single
/// `warning_threshold_pct` instead.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StoredEntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        limit: Option<Decimal>,
        #[serde(default = "default_reset_period")]
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        #[serde(default)]
        thresholds: Vec<u32>,
        warning_threshold_pct: Option<u32>,
        #[serde(default = "default_metered_enabled")]
        enabled: bool,
        #[serde(default)]
        group_by: Option<String>,
    },
    Config {
        value: ConfigValue,
    },
}

impl From<StoredEntitlementValue> for EntitlementValue {
    fn from(stored: StoredEntitlementValue) -> Self {
        match stored {
            StoredEntitlementValue::Boolean { enabled } => Self::Boolean { enabled },
            StoredEntitlementValue::Metered {
                limit,
                reset_period,
                overage_behavior,
                thresholds,
                warning_threshold_pct,
                enabled,
                group_by,
            } => Self::Metered {
                limit,
                reset_period,
                overage_behavior,
                thresholds: merge_thresholds(&thresholds, warning_threshold_pct.as_slice()),
                enabled,
                group_by,
            },
            StoredEntitlementValue::Config { value } => Self::Config { value },
        }
    }
}

/// The thresholds an API request asks for: `thresholds`, sorted and distinct, else the deprecated
/// single `warning_threshold_pct`. Writes validate the result.
pub fn requested_thresholds(
    mut thresholds: Vec<u32>,
    warning_threshold_pct: Option<u32>,
) -> Vec<u32> {
    if thresholds.is_empty() {
        return warning_threshold_pct.into_iter().collect();
    }
    thresholds.sort_unstable();
    thresholds.dedup();
    thresholds
}

/// Both sets of thresholds, ascending and distinct, at most `MAX_THRESHOLDS` of them.
pub fn merge_thresholds(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut merged = a.iter().chain(b).copied().collect::<Vec<_>>();
    merged.sort_unstable();
    merged.dedup();
    merged.truncate(crate::domain::usage_limits::MAX_THRESHOLDS);
    merged
}

impl EntitlementValue {
    /// Without the limit terms that act on nothing, which writes refuse: no limit means Block,
    /// no grace and no thresholds; a sliding window takes no thresholds. For copies of stored
    /// values.
    pub fn without_inert_limit_terms(self) -> Self {
        match self {
            Self::Metered {
                limit,
                reset_period,
                overage_behavior,
                thresholds,
                enabled,
                group_by,
            } => {
                let sliding = matches!(reset_period, ResetPeriod::SlidingWindow { .. });
                Self::Metered {
                    overage_behavior: match limit {
                        Some(_) => overage_behavior,
                        None => OverageBehavior::Block {
                            grace_period_pct: None,
                        },
                    },
                    thresholds: if limit.is_some() && !sliding {
                        thresholds
                    } else {
                        Vec::new()
                    },
                    limit,
                    reset_period,
                    enabled,
                    group_by,
                }
            }
            other => other,
        }
    }
}

/// Repository output: feature type and entitlement value merged into a single variant.
/// No usage data — usage enrichment happens at the service layer.
#[derive(Clone, Debug)]
pub enum ResolvedEntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        metric_id: BillableMetricId,
        limit: Option<Decimal>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        thresholds: Vec<u32>,
        enabled: bool,
        group_by: Option<String>,
    },
    Config {
        value: ConfigValue,
    },
}

/// Service output: resolved value with live usage data embedded in the Metered variant.
/// The variant itself guarantees that metric_id, value, and usage are always co-located.
#[derive(Clone, Debug)]
pub enum EffectiveEntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        metric_id: BillableMetricId,
        limit: Option<Decimal>,
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        thresholds: Vec<u32>,
        /// False once a blocking limit (plus its grace) is reached, even when switched on.
        enabled: bool,
        group_by: Option<String>,
        usage: EntitlementUsage,
        /// Switched on, but refused because a blocking limit (plus its grace) is reached; never
        /// for a grouped limit, which refuses per group.
        limit_reached: bool,
    },
    Config {
        value: ConfigValue,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureProductRef {
    pub id: ProductId,
    pub name: String,
}

/// Feature identity plus the product it belongs to (`None` ⇒ tenant-global feature).
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureRef {
    pub id: FeatureId,
    pub name: String,
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    pub code: String,
    pub product: Option<FeatureProductRef>,
}

/// Wraps an `EntitlementEntityId` with an optional human-readable label, used for display
/// of the resolved origin entity. `None` when the entity has no user-defined name
/// (e.g. subscriptions) or could not be looked up (e.g. deleted entity).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedOrigin {
    pub entity: EntitlementEntityId,
    pub name: Option<String>,
}

/// Output of the pure resolution algorithm — no DB enrichment yet. Carries the origin entity
/// id only; convert to [`ResolvedEntitlement`] via the resolver's `with_origin_names`
/// enrichment step to gain the human-readable origin name.
#[derive(Clone, Debug)]
pub struct RawResolvedEntitlement {
    pub feature: FeatureRef,
    pub value: ResolvedEntitlementValue,
    /// Highest-priority entity that contributed to the final value. See
    /// [`ResolvedEntitlement::origin`] for the resolution semantics.
    pub origin_entity: EntitlementEntityId,
}

/// Result of the resolution algorithm: priorities merged, no usage data, origin enriched
/// with a display name. Construct by enriching a [`RawResolvedEntitlement`] — the
/// non-optional `origin.name` is the type-level signal that enrichment has happened.
#[derive(Clone, Debug)]
pub struct ResolvedEntitlement {
    pub feature: FeatureRef,
    pub value: ResolvedEntitlementValue,
    /// Highest-priority entity that contributed to the final value, with its display name.
    /// For Override winners this is the overriding entity; for Stack merges it is the
    /// highest-priority contributing entity. Feature-level (tenant default) returns
    /// `EntitlementEntityId::Feature(feature_id)`.
    pub origin: ResolvedOrigin,
}

/// Result of the entitlement resolution algorithm for a single feature, with usage enriched.
#[derive(Clone, Debug)]
pub struct EffectiveEntitlement {
    pub feature: FeatureRef,
    pub value: EffectiveEntitlementValue,
    /// Highest-priority entity that contributed to the final value, with its human-readable name.
    /// For Override winners this is the overriding entity; for Stack merges it is the
    /// highest-priority contributing entity. Feature-level (tenant default) returns
    /// `EntitlementEntityId::Feature(feature_id)`.
    pub origin: ResolvedOrigin,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn a_stored_value_without_thresholds_reads_and_writes_unchanged() {
        let legacy = serde_json::json!({
            "type": "metered", "limit": "5", "reset_period": {"type": "never"},
            "overage_behavior": {"type": "allow"}, "enabled": true
        });
        let parsed: EntitlementValue = serde_json::from_value(legacy.clone()).unwrap();
        assert!(matches!(
            &parsed,
            EntitlementValue::Metered { group_by: None, thresholds, .. } if thresholds.is_empty()
        ));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), legacy);
    }

    #[test]
    fn a_stored_warning_threshold_reads_as_a_threshold_and_is_written_as_thresholds() {
        let legacy = serde_json::json!({
            "type": "metered", "limit": "5", "overage_behavior": {"type": "allow"},
            "warning_threshold_pct": 80
        });
        let parsed: EntitlementValue = serde_json::from_value(legacy).unwrap();
        let EntitlementValue::Metered {
            thresholds,
            reset_period,
            enabled,
            ..
        } = &parsed
        else {
            panic!("metered expected");
        };
        assert_eq!(thresholds, &vec![80]);
        assert_eq!(reset_period, &ResetPeriod::Never);
        assert!(*enabled);
        let written = serde_json::to_value(&parsed).unwrap();
        assert_eq!(written["thresholds"], serde_json::json!([80]));
        assert!(written.get("warning_threshold_pct").is_none());

        let both = serde_json::json!({
            "type": "metered", "limit": "5", "overage_behavior": {"type": "allow"},
            "warning_threshold_pct": 90, "thresholds": [50, 90]
        });
        assert!(matches!(
            serde_json::from_value::<EntitlementValue>(both).unwrap(),
            EntitlementValue::Metered { thresholds, .. } if thresholds == vec![50, 90]
        ));
    }

    #[test]
    fn grouped_values_with_thresholds_round_trip() {
        let grouped = EntitlementValue::Metered {
            limit: Some(dec!(5)),
            reset_period: ResetPeriod::BillingCycle,
            overage_behavior: OverageBehavior::Block {
                grace_period_pct: Some(10),
            },
            thresholds: vec![50, 80, 100],
            enabled: true,
            group_by: Some("seat".to_owned()),
        };
        let json = serde_json::to_value(&grouped).unwrap();
        assert_eq!(json["group_by"], "seat");
        assert_eq!(
            serde_json::from_value::<EntitlementValue>(json).unwrap(),
            grouped
        );
    }

    #[test]
    fn merged_thresholds_are_ascending_distinct_and_bounded() {
        assert_eq!(merge_thresholds(&[80, 50], &[50, 100]), vec![50, 80, 100]);
        let many = (1..=8).collect::<Vec<_>>();
        let more = (5..=15).collect::<Vec<_>>();
        assert_eq!(merge_thresholds(&many, &more), (1..=10).collect::<Vec<_>>());
    }
}
