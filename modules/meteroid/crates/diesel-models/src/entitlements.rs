use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::enums::{
    ActorTypeEnum, ConfigValueTypeEnum, EntitlementEffectEnum, EntitlementEntityTypeEnum,
    EntitlementLevelEnum, EntitlementModeEnum, FeatureKindEnum, FeatureLimitCountedByEnum,
    FeatureStatusEnum, FeatureTypeEnum, FeatureValueTypeEnum,
};
use common_domain::ids::{
    AddOnId, BillableMetricId, CustomerId, EntitlementEntityId, EntitlementId, FeatureId, PlanId,
    PlanVersionId, ProductId, QuoteId, ResourceId, ResourceTypeId, SubscriptionId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

/// Product metadata carried alongside a feature row (LEFT JOIN result).
#[derive(Debug)]
pub struct FeatureProductMeta {
    pub id: ProductId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FeatureListOrder {
    #[default]
    Name,
    /// Global features first, then grouped by product name.
    Product,
}

/// A feature row joined with optional product metadata.
/// `product` is `None` for tenant-global features (NULL `product_id`).
#[derive(Debug)]
pub struct FeatureWithProductRow {
    pub feature: FeatureRow,
    pub product: Option<FeatureProductMeta>,
}

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::feature)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FeatureRow {
    pub id: FeatureId,
    pub tenant_id: TenantId,
    pub product_id: Option<ProductId>,
    pub name: String,
    pub description: Option<String>,
    pub feature_type: FeatureTypeEnum,
    pub status: FeatureStatusEnum,
    pub metric_id: Option<BillableMetricId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Unique key used to reference this feature in your code. Cannot be changed after creation.
    pub code: String,
    /// Allowed values for a "select" Config feature. `None` = free-form config value.
    pub config_options: Option<Vec<Option<String>>>,
    /// Authoritative value type of a Config feature. `None` for Boolean/Metered features.
    pub config_value_type: Option<ConfigValueTypeEnum>,
    pub kind: FeatureKindEnum,
    pub value_type: Option<FeatureValueTypeEnum>,
    pub unit: Option<String>,
    pub number_higher_is_better: bool,
    pub select_options: Option<serde_json::Value>,
    pub select_ordered: bool,
    pub limit_counted_by: Option<FeatureLimitCountedByEnum>,
    pub level_resource_type_id: Option<ResourceTypeId>,
    /// `{resets, period?: {unit, interval}, at_limit, grace_pct, thresholds}`; limits only.
    pub policy: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::feature)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FeatureRowNew {
    pub id: FeatureId,
    pub tenant_id: TenantId,
    pub product_id: Option<ProductId>,
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    pub feature_type: FeatureTypeEnum,
    pub status: FeatureStatusEnum,
    pub metric_id: Option<BillableMetricId>,
    pub config_options: Option<Vec<Option<String>>>,
    pub config_value_type: Option<ConfigValueTypeEnum>,
    pub kind: FeatureKindEnum,
    pub value_type: Option<FeatureValueTypeEnum>,
    pub unit: Option<String>,
    pub number_higher_is_better: bool,
    pub select_options: Option<serde_json::Value>,
    pub select_ordered: bool,
    pub limit_counted_by: Option<FeatureLimitCountedByEnum>,
    pub level_resource_type_id: Option<ResourceTypeId>,
    pub policy: Option<serde_json::Value>,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = crate::schema::feature)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FeatureRowPatch {
    // `code` intentionally omitted — it is an immutable addressing key.
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub product_id: Option<Option<ProductId>>,
    pub config_options: Option<Option<Vec<Option<String>>>>,
    pub status: Option<FeatureStatusEnum>,
    pub updated_at: Option<DateTime<Utc>>,
    pub unit: Option<Option<String>>,
    pub number_higher_is_better: Option<bool>,
    pub select_options: Option<Option<serde_json::Value>>,
    pub select_ordered: Option<bool>,
    pub level_resource_type_id: Option<Option<ResourceTypeId>>,
    pub policy: Option<Option<serde_json::Value>>,
}

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::entitlement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntitlementRow {
    pub id: EntitlementId,
    pub tenant_id: TenantId,
    pub feature_id: FeatureId,
    pub entity_id: Uuid,
    pub entity_type: EntitlementEntityTypeEnum,
    pub mode: EntitlementModeEnum,
    pub value: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lineage_id: EntitlementId,
    pub lineage_created_at: DateTime<Utc>,
    /// Set on insert: the earlier of the lineage's first `valid_from` and its creation.
    pub lineage_anchor: DateTime<Utc>,
    pub effect: EntitlementEffectEnum,
    pub level: EntitlementLevelEnum,
    pub level_type_id: Option<ResourceTypeId>,
    pub resource_id: Option<ResourceId>,
    /// `{"<type code>.<attribute>": ["v1", ...]}`.
    pub condition: Option<serde_json::Value>,
    /// Hex sha256 of the canonical condition, `''` without one.
    pub condition_hash: String,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_by_kind: Option<ActorTypeEnum>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<Uuid>,
    pub policy_override: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::entitlement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntitlementRowNew {
    pub id: EntitlementId,
    pub tenant_id: TenantId,
    pub feature_id: FeatureId,
    pub entity_id: Uuid,
    pub entity_type: EntitlementEntityTypeEnum,
    pub mode: EntitlementModeEnum,
    pub value: serde_json::Value,
    pub lineage_id: EntitlementId,
    pub lineage_created_at: DateTime<Utc>,
    pub effect: EntitlementEffectEnum,
    pub level: EntitlementLevelEnum,
    pub level_type_id: Option<ResourceTypeId>,
    pub resource_id: Option<ResourceId>,
    pub condition: Option<serde_json::Value>,
    pub condition_hash: String,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_by_kind: Option<ActorTypeEnum>,
    pub policy_override: Option<serde_json::Value>,
    /// The app's clock, which the fold compares it with: never the database's.
    pub created_at: DateTime<Utc>,
}

impl From<&EntitlementRow> for EntitlementEntityId {
    fn from(value: &EntitlementRow) -> Self {
        match value.entity_type {
            EntitlementEntityTypeEnum::Feature => {
                EntitlementEntityId::Feature(FeatureId::from_const(value.entity_id))
            }
            EntitlementEntityTypeEnum::Customer => {
                EntitlementEntityId::Customer(CustomerId::from_const(value.entity_id))
            }
            EntitlementEntityTypeEnum::PlanVersion => {
                EntitlementEntityId::PlanVersion(PlanVersionId::from_const(value.entity_id))
            }
            EntitlementEntityTypeEnum::AddOn => {
                EntitlementEntityId::AddOn(AddOnId::from_const(value.entity_id))
            }
            EntitlementEntityTypeEnum::Plan => {
                EntitlementEntityId::Plan(PlanId::from_const(value.entity_id))
            }
            EntitlementEntityTypeEnum::Subscription => {
                EntitlementEntityId::Subscription(SubscriptionId::from_const(value.entity_id))
            }
            EntitlementEntityTypeEnum::Quote => {
                EntitlementEntityId::Quote(QuoteId::from_const(value.entity_id))
            }
        }
    }
}
