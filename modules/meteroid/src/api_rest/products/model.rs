use crate::api_rest::model::PaginatedRequest;
use chrono::NaiveDateTime;
use common_domain::ids::{ProductFamilyId, ProductId, string_serde, string_serde_opt};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::api_rest::model::PaginationResponse;

// ── Enums ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProductFeeTypeEnum {
    Rate,
    Slot,
    Capacity,
    Usage,
    ExtraRecurring,
    OneTime,
}

impl From<meteroid_store::domain::enums::FeeTypeEnum> for ProductFeeTypeEnum {
    fn from(value: meteroid_store::domain::enums::FeeTypeEnum) -> Self {
        match value {
            meteroid_store::domain::enums::FeeTypeEnum::Rate => ProductFeeTypeEnum::Rate,
            meteroid_store::domain::enums::FeeTypeEnum::Slot => ProductFeeTypeEnum::Slot,
            meteroid_store::domain::enums::FeeTypeEnum::Capacity => ProductFeeTypeEnum::Capacity,
            meteroid_store::domain::enums::FeeTypeEnum::Usage => ProductFeeTypeEnum::Usage,
            meteroid_store::domain::enums::FeeTypeEnum::ExtraRecurring => {
                ProductFeeTypeEnum::ExtraRecurring
            }
            meteroid_store::domain::enums::FeeTypeEnum::OneTime => ProductFeeTypeEnum::OneTime,
        }
    }
}

impl From<ProductFeeTypeEnum> for meteroid_store::domain::enums::FeeTypeEnum {
    fn from(value: ProductFeeTypeEnum) -> Self {
        match value {
            ProductFeeTypeEnum::Rate => meteroid_store::domain::enums::FeeTypeEnum::Rate,
            ProductFeeTypeEnum::Slot => meteroid_store::domain::enums::FeeTypeEnum::Slot,
            ProductFeeTypeEnum::Capacity => meteroid_store::domain::enums::FeeTypeEnum::Capacity,
            ProductFeeTypeEnum::Usage => meteroid_store::domain::enums::FeeTypeEnum::Usage,
            ProductFeeTypeEnum::ExtraRecurring => {
                meteroid_store::domain::enums::FeeTypeEnum::ExtraRecurring
            }
            ProductFeeTypeEnum::OneTime => meteroid_store::domain::enums::FeeTypeEnum::OneTime,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UsageModelEnum {
    PerUnit,
    Tiered,
    Volume,
    Package,
    Matrix,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SlotUpgradePolicyEnum {
    Prorated,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SlotDowngradePolicyEnum {
    RemoveAtEndOfPeriod,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExtraRecurringBillingTypeEnum {
    Advance,
    Arrears,
}

impl From<UsageModelEnum> for meteroid_store::domain::prices::UsageModel {
    fn from(val: UsageModelEnum) -> Self {
        match val {
            UsageModelEnum::PerUnit => Self::PerUnit,
            UsageModelEnum::Tiered => Self::Tiered,
            UsageModelEnum::Volume => Self::Volume,
            UsageModelEnum::Package => Self::Package,
            UsageModelEnum::Matrix => Self::Matrix,
        }
    }
}

impl From<meteroid_store::domain::prices::UsageModel> for UsageModelEnum {
    fn from(val: meteroid_store::domain::prices::UsageModel) -> Self {
        match val {
            meteroid_store::domain::prices::UsageModel::PerUnit => Self::PerUnit,
            meteroid_store::domain::prices::UsageModel::Tiered => Self::Tiered,
            meteroid_store::domain::prices::UsageModel::Volume => Self::Volume,
            meteroid_store::domain::prices::UsageModel::Package => Self::Package,
            meteroid_store::domain::prices::UsageModel::Matrix => Self::Matrix,
        }
    }
}

impl From<SlotUpgradePolicyEnum> for meteroid_store::domain::price_components::UpgradePolicy {
    fn from(val: SlotUpgradePolicyEnum) -> Self {
        match val {
            SlotUpgradePolicyEnum::Prorated => Self::Prorated,
        }
    }
}

impl From<meteroid_store::domain::price_components::UpgradePolicy> for SlotUpgradePolicyEnum {
    fn from(val: meteroid_store::domain::price_components::UpgradePolicy) -> Self {
        match val {
            meteroid_store::domain::price_components::UpgradePolicy::Prorated => Self::Prorated,
        }
    }
}

impl From<SlotDowngradePolicyEnum> for meteroid_store::domain::price_components::DowngradePolicy {
    fn from(val: SlotDowngradePolicyEnum) -> Self {
        match val {
            SlotDowngradePolicyEnum::RemoveAtEndOfPeriod => Self::RemoveAtEndOfPeriod,
        }
    }
}

impl From<meteroid_store::domain::price_components::DowngradePolicy> for SlotDowngradePolicyEnum {
    fn from(val: meteroid_store::domain::price_components::DowngradePolicy) -> Self {
        match val {
            meteroid_store::domain::price_components::DowngradePolicy::RemoveAtEndOfPeriod => {
                Self::RemoveAtEndOfPeriod
            }
        }
    }
}

impl From<ExtraRecurringBillingTypeEnum> for meteroid_store::domain::enums::BillingType {
    fn from(val: ExtraRecurringBillingTypeEnum) -> Self {
        match val {
            ExtraRecurringBillingTypeEnum::Advance => Self::Advance,
            ExtraRecurringBillingTypeEnum::Arrears => Self::Arrears,
        }
    }
}

impl From<meteroid_store::domain::enums::BillingType> for ExtraRecurringBillingTypeEnum {
    fn from(val: meteroid_store::domain::enums::BillingType) -> Self {
        match val {
            meteroid_store::domain::enums::BillingType::Advance => Self::Advance,
            meteroid_store::domain::enums::BillingType::Arrears => Self::Arrears,
        }
    }
}

// ── FeeStructure (tagged union) ────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RateFeeStructure {}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SlotFeeStructure {
    pub slot_unit_name: String,
    pub upgrade_policy: SlotUpgradePolicyEnum,
    pub downgrade_policy: SlotDowngradePolicyEnum,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct CapacityFeeStructure {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub metric_id: common_domain::ids::BillableMetricId,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct UsageFeeStructure {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub metric_id: common_domain::ids::BillableMetricId,
    pub model: UsageModelEnum,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ExtraRecurringFeeStructure {
    pub billing_type: ExtraRecurringBillingTypeEnum,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct OneTimeFeeStructure {}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProductFeeStructure {
    Rate(RateFeeStructure),
    Slot(SlotFeeStructure),
    Capacity(CapacityFeeStructure),
    Usage(UsageFeeStructure),
    ExtraRecurring(ExtraRecurringFeeStructure),
    OneTime(OneTimeFeeStructure),
}

impl ProductFeeStructure {
    pub fn fee_type_enum(&self) -> meteroid_store::domain::enums::FeeTypeEnum {
        match self {
            Self::Rate(_) => meteroid_store::domain::enums::FeeTypeEnum::Rate,
            Self::Slot(_) => meteroid_store::domain::enums::FeeTypeEnum::Slot,
            Self::Capacity(_) => meteroid_store::domain::enums::FeeTypeEnum::Capacity,
            Self::Usage(_) => meteroid_store::domain::enums::FeeTypeEnum::Usage,
            Self::ExtraRecurring(_) => meteroid_store::domain::enums::FeeTypeEnum::ExtraRecurring,
            Self::OneTime(_) => meteroid_store::domain::enums::FeeTypeEnum::OneTime,
        }
    }
}

impl From<ProductFeeStructure> for meteroid_store::domain::prices::FeeStructure {
    fn from(val: ProductFeeStructure) -> Self {
        match val {
            ProductFeeStructure::Rate(_) => Self::Rate {},
            ProductFeeStructure::Slot(s) => Self::Slot {
                unit_name: s.slot_unit_name,
                upgrade_policy: s.upgrade_policy.into(),
                downgrade_policy: s.downgrade_policy.into(),
            },
            ProductFeeStructure::Capacity(c) => Self::Capacity {
                metric_id: c.metric_id,
            },
            ProductFeeStructure::Usage(u) => Self::Usage {
                metric_id: u.metric_id,
                model: u.model.into(),
            },
            ProductFeeStructure::ExtraRecurring(e) => Self::ExtraRecurring {
                billing_type: e.billing_type.into(),
            },
            ProductFeeStructure::OneTime(_) => Self::OneTime {},
        }
    }
}

impl From<meteroid_store::domain::prices::FeeStructure> for ProductFeeStructure {
    fn from(val: meteroid_store::domain::prices::FeeStructure) -> Self {
        match val {
            meteroid_store::domain::prices::FeeStructure::Rate {} => {
                Self::Rate(RateFeeStructure {})
            }
            meteroid_store::domain::prices::FeeStructure::Slot {
                unit_name,
                upgrade_policy,
                downgrade_policy,
            } => Self::Slot(SlotFeeStructure {
                slot_unit_name: unit_name,
                upgrade_policy: upgrade_policy.into(),
                downgrade_policy: downgrade_policy.into(),
            }),
            meteroid_store::domain::prices::FeeStructure::Capacity { metric_id } => {
                Self::Capacity(CapacityFeeStructure { metric_id })
            }
            meteroid_store::domain::prices::FeeStructure::Usage { metric_id, model } => {
                Self::Usage(UsageFeeStructure {
                    metric_id,
                    model: model.into(),
                })
            }
            meteroid_store::domain::prices::FeeStructure::ExtraRecurring { billing_type } => {
                Self::ExtraRecurring(ExtraRecurringFeeStructure {
                    billing_type: billing_type.into(),
                })
            }
            meteroid_store::domain::prices::FeeStructure::OneTime {} => {
                Self::OneTime(OneTimeFeeStructure {})
            }
        }
    }
}

// ── Response ───────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Product {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ProductId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub fee_type: ProductFeeTypeEnum,
    pub fee_structure: ProductFeeStructure,
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub product_family_id: ProductFamilyId,
    pub catalog: bool,
    #[serde(serialize_with = "crate::api_rest::model::serialize_datetime")]
    pub created_at: NaiveDateTime,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::api_rest::model::serialize_datetime_opt"
    )]
    pub archived_at: Option<NaiveDateTime>,
}

// ── Requests ───────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateProductRequest {
    #[validate(length(min = 1))]
    pub name: String,
    pub description: Option<String>,
    #[serde(with = "string_serde")]
    pub product_family_id: ProductFamilyId,
    pub fee_structure: ProductFeeStructure,
    #[serde(default = "default_true")]
    pub catalog: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateProductRequest {
    #[validate(length(min = 1))]
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub fee_structure: Option<ProductFeeStructure>,
}

// ── List ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProductListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(default, with = "string_serde_opt")]
    pub product_family_id: Option<ProductFamilyId>,
    pub search: Option<String>,
    /// Sort order. Format: `column.direction`. Allowed columns: `name`, `created_at`. Direction: `asc` or `desc`. Default: `name.asc`.
    pub order_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ProductListResponse {
    pub data: Vec<Product>,
    pub pagination_meta: PaginationResponse,
}
