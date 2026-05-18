use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::enums::{
    EntitlementEntityTypeEnum, EntitlementModeEnum, FeatureStatusEnum, FeatureTypeEnum,
};
use common_domain::ids::{
    AddOnId, BillableMetricId, EntitlementEntityId, EntitlementId, FeatureId, PlanId,
    PlanVersionId, ProductId, QuoteId, SubscriptionId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

/// Product metadata carried alongside a feature row (LEFT JOIN result).
#[derive(Debug)]
pub struct FeatureProductMeta {
    pub id: ProductId,
    pub name: String,
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
    pub created_by: Uuid,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::feature)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FeatureRowNew {
    pub id: FeatureId,
    pub tenant_id: TenantId,
    pub product_id: Option<ProductId>,
    pub name: String,
    pub description: Option<String>,
    pub feature_type: FeatureTypeEnum,
    pub status: FeatureStatusEnum,
    pub metric_id: Option<BillableMetricId>,
    pub created_by: Uuid,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = crate::schema::feature)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FeatureRowPatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub product_id: Option<Option<ProductId>>,
    pub status: Option<FeatureStatusEnum>,
    pub updated_at: Option<DateTime<Utc>>,
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
    pub created_by: Uuid,
    pub updated_at: DateTime<Utc>,
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
    pub created_by: Uuid,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = crate::schema::entitlement)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EntitlementRowPatch {
    pub mode: Option<EntitlementModeEnum>,
    pub value: Option<serde_json::Value>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<&EntitlementEntityId> for EntitlementEntityTypeEnum {
    fn from(value: &EntitlementEntityId) -> Self {
        match value {
            EntitlementEntityId::Feature(_) => EntitlementEntityTypeEnum::Feature,
            EntitlementEntityId::PlanVersion(_) => EntitlementEntityTypeEnum::PlanVersion,
            EntitlementEntityId::AddOn(_) => EntitlementEntityTypeEnum::AddOn,
            EntitlementEntityId::Plan(_) => EntitlementEntityTypeEnum::Plan,
            EntitlementEntityId::Subscription(_) => EntitlementEntityTypeEnum::Subscription,
            EntitlementEntityId::Quote(_) => EntitlementEntityTypeEnum::Quote,
        }
    }
}

impl From<&EntitlementRow> for EntitlementEntityId {
    fn from(value: &EntitlementRow) -> Self {
        match value.entity_type {
            EntitlementEntityTypeEnum::Feature => {
                EntitlementEntityId::Feature(FeatureId::from_const(value.entity_id))
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
