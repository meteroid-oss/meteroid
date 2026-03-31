use crate::api_rest::model::PaginatedRequest;
use crate::api_rest::model::PaginationResponse;
use chrono::NaiveDateTime;
use common_domain::ids::{
    AddOnId, BillableMetricId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductFamilyId,
    ProductId, string_serde, string_serde_opt,
};
use o2o::o2o;
use rust_decimal::Decimal;
use serde_enum_str::{Deserialize_enum_str, Serialize_enum_str};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(ToSchema, IntoParams, serde::Serialize, serde::Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct PlanListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(default, with = "string_serde_opt")]
    pub product_family_id: Option<ProductFamilyId>,
    /// Search by plan name
    pub search: Option<String>,
    /// Filter by plan status (can be repeated)
    #[param(inline)]
    #[serde(default)]
    pub status: Vec<PlanStatusEnum>,
    /// Filter by plan type (can be repeated)
    #[param(inline)]
    #[serde(default)]
    pub plan_type: Vec<PlanTypeEnum>,
    /// Sort order. Format: `column.direction`. Allowed columns: `name`, `status`, `plan_type`, `created_at`. Direction: `asc` or `desc`. Default: `created_at.desc`.
    pub order_by: Option<String>,
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
    #[serde(serialize_with = "crate::api_rest::model::serialize_datetime")]
    pub created_at: NaiveDateTime,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub self_service_rank: Option<i32>,
    pub product_family: ProductFamily,

    #[serde(with = "string_serde")]
    pub version_id: PlanVersionId,
    pub version: i32,
    pub currency: String,

    pub net_terms: i32,
    pub billing_cycles: Option<i32>,
    pub period_start_day: Option<i16>,

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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fee: Option<Fee>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "string_serde_opt"
    )]
    pub product_id: Option<ProductId>,
}

/// Recurring rate fee (e.g., monthly subscription)
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct RatePlanFee {
    pub rates: Vec<TermRate>,
}

/// Slot-based fee (e.g., per-seat pricing)
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct SlotPlanFee {
    pub rates: Vec<TermRate>,
    pub slot_unit_name: String,
    pub minimum_count: Option<u32>,
    pub quota: Option<u32>,
}

/// Capacity-based fee with included committed usage and overage
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct CapacityPlanFee {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub thresholds: Vec<CapacityThreshold>,
    pub cadence: BillingPeriodEnum,
}

/// Usage-based fee
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct UsagePlanFee {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub pricing: UsagePricingModel,
    pub cadence: BillingPeriodEnum,
}

/// Extra recurring fee
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct ExtraRecurringPlanFee {
    pub unit_price: Decimal,
    pub quantity: u32,
    pub billing_type: BillingType,
    pub cadence: BillingPeriodEnum,
}

/// One-time fee
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct OneTimePlanFee {
    pub unit_price: Decimal,
    pub quantity: u32,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Fee {
    Rate(RatePlanFee),
    Slot(SlotPlanFee),
    Capacity(CapacityPlanFee),
    Usage(UsagePlanFee),
    ExtraRecurring(ExtraRecurringPlanFee),
    OneTime(OneTimePlanFee),
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
pub struct PerUnitPlanPricing {
    pub rate: Decimal,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct TieredPlanPricing {
    pub tiers: Vec<TierRow>,
    pub block_size: Option<u64>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct VolumePlanPricing {
    pub tiers: Vec<TierRow>,
    pub block_size: Option<u64>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct PackagePlanPricing {
    pub block_size: u64,
    pub rate: Decimal,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct MatrixPlanPricing {
    pub rates: Vec<MatrixRow>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(as = PlanUsagePricingModel)]
pub enum UsagePricingModel {
    PerUnit(PerUnitPlanPricing),
    Tiered(TieredPlanPricing),
    Volume(VolumePlanPricing),
    Package(PackagePlanPricing),
    Matrix(MatrixPlanPricing),
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

pub use crate::api_rest::model::BillingPeriodEnum;

#[derive(
    o2o, Copy, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq,
)]
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

// ── Input types ────────────────────────────────────────────────

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct BillingConfig {
    #[serde(default)]
    pub net_terms: i32,
    pub billing_cycles: Option<i32>,
    pub period_start_day: Option<i16>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
#[validate(schema(function = "validate_create_plan"))]
pub struct CreatePlanRequest {
    #[validate(length(min = 1, max = 255))]
    pub name: String,
    pub description: Option<String>,
    #[serde(with = "string_serde")]
    pub product_family_id: ProductFamilyId,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub currency: String,
    pub self_service_rank: Option<i32>,
    pub billing: Option<BillingConfig>,
    pub trial: Option<TrialConfig>,
    pub components: Vec<PriceComponentInput>,
    #[serde(default)]
    pub add_ons: Vec<PlanAddOnInput>,
}

fn validate_create_plan(req: &CreatePlanRequest) -> Result<(), validator::ValidationError> {
    if req.plan_type != PlanTypeEnum::Free && req.components.is_empty() {
        return Err(validator::ValidationError::new("components_required")
            .with_message("Non-free plans must have at least one price component".into()));
    }
    Ok(())
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct ReplacePlanRequest {
    #[validate(length(min = 1, max = 255))]
    pub name: String,
    pub description: Option<String>,
    pub status: Option<PlanStatusEnum>,
    pub currency: String,
    pub billing: Option<BillingConfig>,
    pub trial: Option<TrialConfig>,
    pub components: Vec<PriceComponentInput>,
    #[serde(default)]
    pub add_ons: Vec<PlanAddOnInput>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct PatchPlanRequest {
    #[validate(length(min = 1, max = 255))]
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub self_service_rank: Option<Option<i32>>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct PriceComponentInput {
    pub name: String,
    #[serde(default, with = "string_serde_opt")]
    pub product_id: Option<ProductId>,
    pub fee: Fee,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct PlanAddOnInput {
    #[serde(with = "string_serde")]
    pub add_on_id: AddOnId,
    #[serde(default, with = "string_serde_opt")]
    pub price_id: Option<PriceId>,
    pub self_serviceable: Option<bool>,
    pub max_instances: Option<i32>,
}

// ── Version and query types ────────────────────────────────────

#[derive(ToSchema, IntoParams, serde::Serialize, serde::Deserialize, Default)]
#[into_params(parameter_in = Query)]
pub struct PlanGetQuery {
    /// Filter by version: "draft", a version number, or omitted for active
    pub version: Option<String>,
}

#[derive(ToSchema, IntoParams, serde::Serialize, serde::Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct PlanVersionListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PlanVersionSummary {
    #[serde(with = "string_serde")]
    pub id: PlanVersionId,
    pub version: i32,
    pub is_draft: bool,
    pub currency: String,
    #[serde(serialize_with = "crate::api_rest::model::serialize_datetime")]
    pub created_at: NaiveDateTime,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PlanVersionListResponse {
    pub data: Vec<PlanVersionSummary>,
    pub pagination_meta: PaginationResponse,
}
