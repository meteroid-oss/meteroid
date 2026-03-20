use crate::api_rest::model::PaginatedRequest;
use chrono::NaiveDateTime;
use common_domain::ids::{ProductFamilyId, ProductId};
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

// ── FeeStructure (tagged union) ────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "fee_type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProductFeeStructure {
    Rate {},
    Slot {
        slot_unit_name: String,
        upgrade_policy: SlotUpgradePolicyEnum,
        downgrade_policy: SlotDowngradePolicyEnum,
    },
    Capacity {
        #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
        metric_id: common_domain::ids::BillableMetricId,
    },
    Usage {
        #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
        metric_id: common_domain::ids::BillableMetricId,
        model: UsageModelEnum,
    },
    ExtraRecurring {
        billing_type: ExtraRecurringBillingTypeEnum,
    },
    OneTime {},
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
    pub product_family_id: Option<ProductFamilyId>,
    pub search: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ProductListResponse {
    pub data: Vec<Product>,
    pub pagination_meta: PaginationResponse,
}
