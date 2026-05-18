use crate::domain::enums::{EntitlementModeEnum, FeatureStatusEnum};
use crate::errors::{StoreError, StoreErrorReport};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, BillableMetricId, EntitlementEntityId, EntitlementId, FeatureId, ProductId, TenantId,
};
use diesel_models::entitlements::{EntitlementRow, FeatureRowNew, FeatureWithProductRow};
use diesel_models::enums::{
    FeatureStatusEnum as DbFeatureStatusEnum, FeatureTypeEnum as DbFeatureTypeEnum,
};
use rust_decimal::Decimal;
use uuid::Uuid;

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

#[derive(Clone, Debug)]
pub enum FeatureType {
    Boolean,
    Metered { metric_id: BillableMetricId },
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
    pub description: Option<String>,
    pub feature_type: FeatureType,
    pub status: FeatureStatusEnum,
    pub created_at: DateTime<Utc>,
    pub created_by: Uuid,
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
        };
        Ok(Feature {
            id: feature.id,
            tenant_id: feature.tenant_id,
            product: product.map(|p| FeatureProductRef {
                id: p.id,
                name: p.name,
            }),
            name: feature.name,
            description: feature.description,
            feature_type,
            status: feature.status.into(),
            created_at: feature.created_at,
            created_by: feature.created_by,
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
    pub description: Option<String>,
    pub feature_type: FeatureType,
    pub created_by: Uuid,
    pub entitlement: Option<FeatureEntitlementSpec>,
}

impl From<FeatureNew> for FeatureRowNew {
    fn from(f: FeatureNew) -> Self {
        let FeatureNew {
            tenant_id,
            product_id,
            name,
            description,
            feature_type,
            created_by,
            entitlement: _,
        } = f;
        let (feature_type_enum, metric_id) = match feature_type {
            FeatureType::Boolean => (DbFeatureTypeEnum::Boolean, None),
            FeatureType::Metered { metric_id } => (DbFeatureTypeEnum::Metered, Some(metric_id)),
        };
        FeatureRowNew {
            id: FeatureId::new(),
            tenant_id,
            product_id,
            name,
            description,
            feature_type: feature_type_enum,
            status: DbFeatureStatusEnum::Active,
            metric_id,
            created_by,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FeatureUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub product_id: Option<Option<ProductId>>,
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
    pub created_by: Uuid,
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
            created_by: row.created_by,
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

/// Caller-provided entitlement spec for feature creation: feature_id is implicit
/// (the feature being created), caller supplies which entity receives the entitlement.
#[derive(Clone, Debug)]
pub struct FeatureEntitlementSpec {
    pub entity: EntitlementEntityId,
    pub value: EntitlementValue,
}

#[derive(Clone, Debug)]
pub struct EntitlementNew {
    pub tenant_id: TenantId,
    pub feature_id: FeatureId,
    pub entity: EntitlementEntityId,
    pub value: EntitlementValue,
    pub created_by: Uuid,
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntitlementValue {
    Boolean {
        enabled: bool,
    },
    Metered {
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<Decimal>,
        #[serde(default = "default_reset_period")]
        reset_period: ResetPeriod,
        overage_behavior: OverageBehavior,
        #[serde(skip_serializing_if = "Option::is_none")]
        warning_threshold_pct: Option<u32>,
        #[serde(default = "default_metered_enabled")]
        enabled: bool,
    },
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
        warning_threshold_pct: Option<u32>,
        enabled: bool,
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
        warning_threshold_pct: Option<u32>,
        enabled: bool,
        usage: EntitlementUsage,
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
    pub created_at: DateTime<Utc>,
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
    /// Earliest creation time across resolved entitlements. Used as default usage floor.
    pub created_at: DateTime<Utc>,
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
    pub created_at: DateTime<Utc>,
    /// Highest-priority entity that contributed to the final value, with its human-readable name.
    /// For Override winners this is the overriding entity; for Stack merges it is the
    /// highest-priority contributing entity. Feature-level (tenant default) returns
    /// `EntitlementEntityId::Feature(feature_id)`.
    pub origin: ResolvedOrigin,
}
