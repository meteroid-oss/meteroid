use crate::api_rest::model::PaginatedRequest;
use crate::api_rest::model::PaginationResponse;
use chrono::NaiveDateTime;
use common_domain::ids::{
    BillableMetricId, PlanId, PlanVersionId, PriceComponentId, ProductFamilyId, ProductId,
    string_serde, string_serde_opt,
};
use o2o::o2o;
use rust_decimal::Decimal;
use serde_enum_str::{Deserialize_enum_str, Serialize_enum_str};
use std::collections::HashMap;
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct PlanListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(default, with = "string_serde_opt")]
    pub product_family_id: Option<ProductFamilyId>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct ProductFamily {
    #[serde(with = "string_serde")]
    pub id: ProductFamilyId,
    pub name: String,
}

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::PlanTypeEnum)]
pub enum PlanTypeEnum {
    Standard,
    Free,
    Custom,
}

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::PlanStatusEnum)]
pub enum PlanStatusEnum {
    Draft,
    Active,
    Inactive,
    Archived,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    #[serde(with = "string_serde")]
    pub id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub product_family: ProductFamily,

    #[serde(with = "string_serde")]
    pub version_id: PlanVersionId,
    pub version: i32,
    pub currency: String,

    pub net_terms: i32,

    pub trial: Option<TrialConfig>,

    pub price_components: Vec<PriceComponent>,

    pub available_parameters: AvailableParameters,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct TrialConfig {
    pub duration_days: u32,
    pub is_free: bool,
    #[serde(with = "string_serde_opt")]
    pub trialing_plan_id: Option<PlanId>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug, Default)]
pub struct AvailableParameters {
    /// Map of component_id -> available billing periods (e.g., "MONTHLY", "ANNUAL")
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub billing_periods: HashMap<String, Vec<BillingPeriodEnum>>,

    /// Map of component_id -> available capacity values
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub capacity_thresholds: HashMap<String, Vec<u64>>,

    /// List of component_ids that support slot parametrization (initial slot count)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub slot_components: Vec<String>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct PriceComponent {
    #[serde(with = "string_serde")]
    pub id: PriceComponentId,
    pub name: String,
    pub fee: Fee,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "string_serde_opt"
    )]
    pub product_id: Option<ProductId>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "fee_type", rename_all = "snake_case")]
pub enum Fee {
    /// Recurring rate fee (e.g., monthly subscription)
    Rate { rates: Vec<TermRate> },
    /// Slot-based fee (e.g., per-seat pricing)
    Slot {
        rates: Vec<TermRate>,
        slot_unit_name: String,
        minimum_count: Option<u32>,
        quota: Option<u32>,
    },
    /// Capacity-based fee with included committed usage and overage
    Capacity {
        #[serde(with = "string_serde")]
        metric_id: BillableMetricId,
        thresholds: Vec<CapacityThreshold>,
        cadence: BillingPeriodEnum,
    },
    /// Usage-based fee
    Usage {
        #[serde(with = "string_serde")]
        metric_id: BillableMetricId,
        pricing: UsagePricingModel,
        cadence: BillingPeriodEnum,
    },
    /// Extra recurring fee
    ExtraRecurring {
        unit_price: Decimal,
        quantity: u32,
        billing_type: BillingType,
        cadence: BillingPeriodEnum,
    },
    /// One-time fee
    OneTime { unit_price: Decimal, quantity: u32 },
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct TermRate {
    pub term: BillingPeriodEnum,
    pub price: Decimal,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct CapacityThreshold {
    pub included_amount: u64,
    pub price: Decimal,
    pub per_unit_overage: Decimal,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum UsagePricingModel {
    PerUnit {
        rate: Decimal,
    },
    Tiered {
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Volume {
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Package {
        block_size: u64,
        rate: Decimal,
    },
    Matrix {
        rates: Vec<MatrixRow>,
    },
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct TierRow {
    pub first_unit: u64,
    pub rate: Decimal,
    pub flat_fee: Option<Decimal>,
    pub flat_cap: Option<Decimal>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct MatrixRow {
    pub dimension1: MatrixDimension,
    pub dimension2: Option<MatrixDimension>,
    pub per_unit_price: Decimal,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct MatrixDimension {
    pub key: String,
    pub value: String,
}

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::BillingPeriodEnum)]
pub enum BillingPeriodEnum {
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::BillingType)]
pub enum BillingType {
    Advance,
    Arrears,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PlanListResponse {
    pub data: Vec<Plan>,
    pub pagination_meta: PaginationResponse,
}
