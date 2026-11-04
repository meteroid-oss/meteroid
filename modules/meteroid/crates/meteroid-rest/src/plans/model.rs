use crate::currencies::model::Currency;
use crate::entitlements::model::{
    Entitlement, EntitlementSpecRequest, validate_unique_feature_ids,
};
use common_domain::ids::{
    AddOnId, BillableMetricId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductFamilyId,
    ProductId, string_serde, string_serde_opt,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::PaginatedRequest;
use meteroid_rest_common::model::PaginationResponse;
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

pub use crate::productfamilies::model::ProductFamily;

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::PlanTypeEnum)]
#[schema(title = "PlanType")]
pub enum PlanTypeEnum {
    Standard,
    Free,
    Custom,
}

#[derive(o2o, Clone, ToSchema, Deserialize_enum_str, Serialize_enum_str, Debug, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::PlanStatusEnum)]
#[schema(title = "PlanStatus")]
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
    pub created_at: ApiDateTime,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub self_service_rank: Option<i32>,
    /// Subscriptions to this plan are issued a software license automatically.
    #[serde(default)]
    #[schema(required = false)]
    pub issues_license: bool,
    pub product_family: ProductFamily,

    #[serde(with = "string_serde")]
    pub version_id: PlanVersionId,
    pub version: i32,
    pub currency: Currency,
    /// The plan's amounts are quoted tax-included ("9.99 incl. VAT"): tax is carved out of
    /// them at invoice time instead of being added on top, so the customer pays the quoted
    /// price whatever rate applies. A customer who bears no tax (reverse charge, exempt,
    /// export) still pays it in full. Defaults to `false`.
    pub tax_inclusive: bool,

    pub net_terms: i32,
    pub billing_cycles: Option<i32>,
    pub period_start_day: Option<i16>,

    pub trial: Option<TrialConfig>,

    pub price_components: Vec<PriceComponent>,

    pub available_parameters: AvailableParameters,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub entitlements: Vec<Entitlement>,

    /// Plan-level minimum commitment. When a billing period's charges in scope, before discounts,
    /// total less than `minimum_commitment.amount`, the invoice that closes the period adds a
    /// "Minimum commitment" true-up line for the difference. A partial period is prorated by day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_commitment: Option<MinimumCommitment>,

    /// What usage does once the credit it draws on runs out. Absent on a draft whose rule was not
    /// chosen yet: publishing sets the default (`pause` when the version grants credit or prices
    /// usage in a credit unit, `invoice` otherwise).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_exhausted_rule: Option<CreditExhaustedRule>,
}

/// What usage does once the credit it draws on runs out, for every denomination (currency or
/// credit unit) the plan charges usage in. A customer's wallet can override it.
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CreditExhaustedRule {
    /// Usage beyond the credit is invoiced.
    Invoice,
    /// Access checks refuse usage once the credit the subscription can spend is used up.
    Pause,
    /// Usage beyond the credit is invoiced up to a cap per billing period, then refused.
    InvoiceUpTo(CreditCaps),
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct CreditCaps {
    /// One per denomination; usage charged in a denomination without a cap pauses at zero.
    pub caps: Vec<CreditCap>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct CreditCap {
    /// A currency or credit unit code.
    pub denomination: String,
    /// Usage beyond the credit invoiced per billing period, in minor units of `denomination`.
    pub amount: i64,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct MinimumCommitment {
    /// Decimal string in the plan currency, e.g. "100.00".
    pub amount: String,
    pub scope: MinimumCommitmentScope,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MinimumCommitmentScope {
    AllComponents(AllComponentsScope),
    Products(ProductsScope),
}

/// Every revenue line counts toward the floor.
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct AllComponentsScope {}

/// Only lines for the listed products count. A product is the identity shared by plan
/// components, overrides and ad-hoc extras, so a subscription's billed set is matched uniformly.
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct ProductsScope {
    pub product_ids: Vec<String>,
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
    /// Prices the overage in a credit unit: `per_unit_overage` is then a number of credits, drawn
    /// from the customer's balance in that unit before anything is billed in money. Absent prices
    /// the overage in the plan currency. A replacement (`PUT`) must send it again to keep it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<PlanFeeCreditUnit>,
}

/// The credit unit a plan's usage price is expressed in, with its value in the plan currency.
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct PlanFeeCreditUnit {
    /// An active credit unit of the tenant (`GET /api/v1/credit-units`).
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub credit_unit_id: common_domain::ids::CreditUnitId,
    /// Plan-currency major units per credit (e.g. "0.01"), positive: the money billed for each
    /// credit the customer's balance in the unit does not cover, before tax.
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
}

/// Usage-based fee
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct UsagePlanFee {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub pricing: UsagePricingModel,
    pub cadence: BillingPeriodEnum,
    /// Prices the usage in a credit unit: `pricing` amounts are then numbers of credits, drawn
    /// from the customer's balance in that unit before anything is billed in money. Absent prices
    /// the usage in the plan currency. A replacement (`PUT`) must send it again to keep it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<PlanFeeCreditUnit>,
}

/// Extra recurring fee
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct ExtraRecurringPlanFee {
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: Decimal,
    pub quantity: u32,
    pub billing_type: BillingType,
    pub cadence: BillingPeriodEnum,
}

/// One-time fee
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct OneTimePlanFee {
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: Decimal,
    pub quantity: u32,
}

/// Credit pack: charged `unit_price` × `quantity` in advance each billing period, it grants
/// `credit_amount` × `quantity` of credit, in the plan currency or in `credit_unit_id`, once paid.
/// A zero `unit_price` makes it credit included with the plan (promotional). A currency pack
/// priced below its credit (e.g. 500.00 buys 550.00) grants the price paid as paid credit and the
/// difference as promotional bonus credit, which applies before tax.
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct CreditPlanFee {
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: Decimal,
    pub quantity: u32,
    #[schema(value_type = String, format = "decimal")]
    pub credit_amount: Decimal,
    pub cadence: BillingPeriodEnum,
    /// DAY and MONTH run from payment; BILLING_PERIOD ends with the invoiced period.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<CreditPackExpiry>,
    /// Included packs only (unit_price 0) that expire: unspent credit is carried once into a new
    /// grant, spent before the next period's credit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollover: Option<CreditPackRollover>,
    /// The credit unit `credit_amount` is granted in; absent grants the plan currency. The pack is
    /// paid in the plan currency, and taxed when sold if it grants a credit unit.
    #[serde(
        default,
        with = "string_serde_opt",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    pub credit_unit_id: Option<common_domain::ids::CreditUnitId>,
    /// When true, the pack is billed and its credit granted once, on the subscription's first
    /// invoice, instead of every period (e.g. a welcome credit). A plan change grants it on the
    /// next invoice only if the subscription never received a one-time credit of the same
    /// product; credit packs billed every period don't count. Cannot be combined with `rollover`.
    #[serde(default)]
    pub once: bool,
}

/// Unspent included credit carried once into a new grant when it expires, spent before the next
/// period's credit.
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct CreditPackRollover {
    /// Cap per pack, in the pack's credit; absent carries all that is left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub max_amount: Option<Decimal>,
    /// Term of the rolled credit from the original expiry: DAY or MONTH.
    pub expiry: CreditPackExpiry,
}

/// How long funded credit stays spendable. BILLING_PERIOD takes value 1.
#[derive(Clone, Copy, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct CreditPackExpiry {
    /// DAY, MONTH or BILLING_PERIOD.
    pub unit: CreditExpiryUnit,
    /// How many days or months; 1 for BILLING_PERIOD.
    pub value: u32,
}

#[derive(Clone, Copy, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditExpiryUnit {
    Day,
    Month,
    BillingPeriod,
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
    Credit(CreditPlanFee),
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct TermRate {
    pub term: BillingPeriodEnum,
    #[schema(value_type = String, format = "decimal")]
    pub price: Decimal,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct CapacityThreshold {
    pub included_amount: u64,
    #[schema(value_type = String, format = "decimal")]
    pub price: Decimal,
    #[schema(value_type = String, format = "decimal")]
    pub per_unit_overage: Decimal,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct PerUnitPlanPricing {
    #[schema(value_type = String, format = "decimal")]
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
    #[schema(value_type = String, format = "decimal")]
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

#[derive(o2o, Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[map_owned(meteroid_store::domain::price_components::TierRow)]
pub struct TierRow {
    pub first_unit: u64,
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_fee: Option<Decimal>,
    #[schema(value_type = Option<String>, format = "decimal")]
    pub flat_cap: Option<Decimal>,
}

#[derive(o2o, Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[map_owned(meteroid_store::domain::price_components::MatrixRow)]
pub struct MatrixRow {
    #[map(~.into())]
    pub dimension1: MatrixDimension,
    #[map(~.map(| x | x.into()))]
    pub dimension2: Option<MatrixDimension>,
    #[schema(value_type = String, format = "decimal")]
    pub per_unit_price: Decimal,
}

#[derive(o2o, Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[map_owned(meteroid_store::domain::price_components::MatrixDimension)]
pub struct MatrixDimension {
    pub key: String,
    pub value: String,
}

pub use meteroid_rest_common::model::BillingPeriodEnum;

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
    /// The plan's amounts are quoted tax-included ("9.99 incl. VAT"): tax is carved out of
    /// them at invoice time instead of being added on top, so the customer pays the quoted
    /// price whatever rate applies. A customer who bears no tax (reverse charge, exempt,
    /// export) still pays it in full. Defaults to `false`.
    #[serde(default)]
    pub tax_inclusive: bool,
    /// What usage does once the credit it draws on runs out. Omitted, the version takes the
    /// default when it is published: `pause` when it grants credit or prices usage in a credit
    /// unit, `invoice` otherwise.
    #[serde(default)]
    pub credit_exhausted_rule: Option<CreditExhaustedRule>,
    pub self_service_rank: Option<i32>,
    pub billing: Option<BillingConfig>,
    pub trial: Option<TrialConfig>,
    pub components: Vec<PriceComponentInput>,
    #[serde(default)]
    pub add_ons: Vec<PlanAddOnInput>,
    /// Entitlements to attach to this plan's version. Replacing a published plan creates a
    /// new version, and entitlements belong to a version, so passing them here keeps them
    /// attached to whichever version the call produces.
    #[serde(default)]
    #[validate(nested, custom(function = "validate_unique_feature_ids"))]
    pub entitlements: Vec<EntitlementSpecRequest>,
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
    /// The plan's amounts are quoted tax-included ("9.99 incl. VAT"): tax is carved out of
    /// them at invoice time instead of being added on top, so the customer pays the quoted
    /// price whatever rate applies. A customer who bears no tax (reverse charge, exempt,
    /// export) still pays it in full. Defaults to `false`.
    #[serde(default)]
    pub tax_inclusive: bool,
    /// What usage does once the credit it draws on runs out. Omitted, the version takes the
    /// default when it is published: `pause` when it grants credit or prices usage in a credit
    /// unit, `invoice` otherwise.
    #[serde(default)]
    pub credit_exhausted_rule: Option<CreditExhaustedRule>,
    pub billing: Option<BillingConfig>,
    pub trial: Option<TrialConfig>,
    /// Entitlements to attach to this plan's version. Replacing a published plan creates a
    /// new version, and entitlements belong to a version, so passing them here keeps them
    /// attached to whichever version the call produces.
    #[serde(default)]
    #[validate(nested, custom(function = "validate_unique_feature_ids"))]
    pub entitlements: Vec<EntitlementSpecRequest>,
    pub components: Vec<PriceComponentInput>,
    #[serde(default)]
    pub add_ons: Vec<PlanAddOnInput>,
    /// Optional plan-level period minimum commitment. Component scope must reference names
    /// that exist in `components` (matched by name, not by ID, since component IDs are
    /// assigned by the server). Use `MinimumCommitmentInput` so the client can express
    /// scope by component name.
    #[serde(default)]
    pub minimum_commitment: Option<MinimumCommitmentInput>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct MinimumCommitmentInput {
    /// Decimal string in the plan currency.
    pub amount: String,
    pub scope: MinimumCommitmentInputScope,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MinimumCommitmentInputScope {
    AllComponents(AllComponentsScope),
    Components(ComponentsScope),
}

/// Component names — matched against `ReplacePlanRequest::components[].name`.
#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct ComponentsScope {
    pub component_names: Vec<String>,
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
    pub currency: Currency,
    pub created_at: ApiDateTime,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PlanVersionListResponse {
    pub data: Vec<PlanVersionSummary>,
    pub pagination_meta: PaginationResponse,
}
