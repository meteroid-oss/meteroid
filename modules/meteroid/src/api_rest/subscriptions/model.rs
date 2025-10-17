use crate::api_rest::currencies::model::Currency;
use crate::api_rest::model::{BillingPeriodEnum, PaginatedRequest, PaginationResponse};
use chrono::NaiveDate;
use common_domain::ids::{
    AddOnId, AppliedCouponId, BillableMetricId, CouponId, CustomerId, PlanVersionId,
    PriceComponentId, ProductId,
};
use common_domain::ids::{PlanId, string_serde_opt, string_serde_vec_opt};
use common_domain::ids::{SubscriptionId, string_serde};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::Validate;

#[derive(ToSchema, Serialize, Deserialize, Validate)]
pub struct SubscriptionRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(default, with = "string_serde_opt")]
    pub customer_id: Option<CustomerId>,
    #[serde(default, with = "string_serde_opt")]
    pub plan_id: Option<PlanId>,
    pub status: Option<Vec<SubscriptionStatusEnum>>,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct Subscription {
    #[serde(with = "string_serde")]
    pub id: SubscriptionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day_anchor: i16,
    pub currency: Currency,
    #[serde(with = "string_serde")]
    pub plan_id: PlanId,
    pub plan_name: String,
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    pub plan_version: u32,
    pub status: SubscriptionStatusEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub purchase_order: Option<String>,
    /// If false, invoices will stay in Draft until manually reviewed and finalized. Default to true.
    pub auto_advance_invoices: bool,
    /// Automatically try to charge the customer's configured payment method on finalize.
    pub charge_automatically: bool,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CouponDiscount {
    Percentage {
        #[schema(value_type = String, format = "decimal")]
        percentage: rust_decimal::Decimal,
    },
    Fixed {
        currency: String,
        #[schema(value_type = String, format = "decimal")]
        amount: rust_decimal::Decimal,
    },
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct Coupon {
    #[serde(with = "string_serde")]
    pub id: CouponId,
    pub code: String,
    pub description: String,
    pub discount: CouponDiscount,
    pub expires_at: Option<chrono::NaiveDateTime>,
    pub redemption_limit: Option<i32>,
    pub recurring_value: Option<i32>,
    pub reusable: bool,
    pub disabled: bool,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct AppliedCoupon {
    #[serde(with = "string_serde")]
    pub id: AppliedCouponId,
    #[serde(with = "string_serde")]
    pub coupon_id: CouponId,
    pub is_active: bool,
    #[schema(value_type = Option<String>, format = "decimal")]
    pub applied_amount: Option<rust_decimal::Decimal>,
    pub applied_count: Option<i32>,
    pub last_applied_at: Option<chrono::NaiveDateTime>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct AppliedCouponDetailed {
    pub coupon: Coupon,
    pub applied_coupon: AppliedCoupon,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct SubscriptionDetails {
    #[serde(with = "string_serde")]
    pub id: SubscriptionId,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    pub customer_name: String,
    pub customer_alias: Option<String>,
    pub billing_day_anchor: i16,
    pub currency: Currency,
    #[serde(with = "string_serde")]
    pub plan_id: PlanId,
    pub plan_name: String,
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    pub plan_version: u32,
    pub status: SubscriptionStatusEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub purchase_order: Option<String>,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub components: Vec<SubscriptionComponent>,
    pub add_ons: Vec<SubscriptionAddOn>,
    pub applied_coupons: Vec<AppliedCouponDetailed>,
    pub checkout_url: Option<String>,
}

#[derive(ToSchema, Serialize, Deserialize, Validate, Debug)]
pub struct SubscriptionCreateRequest {
    #[serde(with = "string_serde")]
    pub plan_id: PlanId,
    #[schema(nullable = false)]
    pub version: Option<i32>,
    #[schema(format = "CustomerId or customer alias")]
    pub customer_id_or_alias: String,
    #[schema(nullable = false)]
    pub trial_days: Option<u32>,
    #[schema(example = "2024-11-01")]
    pub start_date: NaiveDate,
    #[schema(example = "2025-11-01")]
    #[schema(nullable = false)]
    pub end_date: Option<NaiveDate>,
    #[validate(range(min = 1, max = 32767))]
    #[schema(minimum = 1, maximum = 32767)]
    pub billing_day_anchor: Option<u16>,
    #[schema(nullable = false)]
    pub net_terms: Option<u32>,
    #[schema(nullable = false)]
    pub invoice_memo: Option<String>,

    #[schema(nullable = false)]
    pub coupon_codes: Option<Vec<String>>,
    #[schema(nullable = false)]
    pub activation_condition: SubscriptionActivationConditionEnum,
    #[schema(nullable = false)]
    pub price_components: Option<CreateSubscriptionComponents>,
    #[schema(nullable = false)]
    pub add_ons: Option<Vec<CreateSubscriptionAddOn>>,
    pub purchase_order: Option<String>,
    #[schema(nullable = false)]
    pub auto_advance_invoices: Option<bool>,
    #[schema(nullable = false)]
    pub charge_automatically: Option<bool>,
    // #[schema(value_type = Option<String>, format = "decimal")]
    // #[schema(nullable = false)]
    // pub invoice_threshold: Option<rust_decimal::Decimal>,
}

#[derive(o2o, Clone, ToSchema, Serialize, Deserialize, Debug)]
#[map_owned(meteroid_store::domain::enums::SubscriptionActivationCondition)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionActivationConditionEnum {
    OnStart,
    OnCheckout,
    Manual,
}

#[derive(o2o, Clone, ToSchema, Serialize, Deserialize, Debug)]
#[map_owned(meteroid_store::domain::enums::SubscriptionFeeBillingPeriod)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionFeeBillingPeriodEnum {
    OneTime,
    Monthly,
    Quarterly,
    Semiannual,
    Annual,
}

#[derive(o2o, Clone, ToSchema, Serialize, Deserialize, Debug)]
#[map_owned(meteroid_store::domain::enums::SubscriptionStatusEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionStatusEnum {
    PendingActivation,
    PendingCharge,
    TrialActive,
    Active,
    TrialExpired,
    Paused,
    Suspended,
    Cancelled,
    Completed,
    Superseded,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CreateSubscriptionComponents {
    pub parameterized_components: Option<Vec<ComponentParameterization>>,
    pub overridden_components: Option<Vec<ComponentOverride>>,
    pub extra_components: Option<Vec<ExtraComponent>>,
    #[serde(default, with = "string_serde_vec_opt")]
    pub remove_components: Option<Vec<PriceComponentId>>,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::subscription_components::SubscriptionFee)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionFee {
    Rate {
        #[schema(value_type = String, format = "decimal")]
        rate: rust_decimal::Decimal,
    },
    OneTime {
        #[schema(value_type = String, format = "decimal")]
        rate: rust_decimal::Decimal,
        quantity: u32,
    },
    Recurring {
        #[schema(value_type = String, format = "decimal")]
        rate: rust_decimal::Decimal,
        quantity: u32,
        #[map(~.into())]
        billing_type: BillingTypeEnum,
    },
    Capacity {
        #[schema(value_type = String, format = "decimal")]
        rate: rust_decimal::Decimal,
        included: u64,
        #[schema(value_type = String, format = "decimal")]
        overage_rate: rust_decimal::Decimal,
        #[serde(with = "string_serde")]
        metric_id: BillableMetricId,
    },
    Slot {
        unit: String,
        #[schema(value_type = String, format = "decimal")]
        unit_rate: rust_decimal::Decimal,
        min_slots: Option<u32>,
        max_slots: Option<u32>,
        initial_slots: u32,
    },
    Usage {
        #[serde(with = "string_serde")]
        metric_id: BillableMetricId,
        #[map(~.into())]
        model: UsagePricingModel,
    },
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::price_components::MatrixRow)]
pub struct MatrixRow {
    #[map(~.into())]
    pub dimension1: MatrixDimension,
    #[map(~.map(| x | x.into()))]
    pub dimension2: Option<MatrixDimension>,
    #[schema(value_type = String, format = "decimal")]
    pub per_unit_price: rust_decimal::Decimal,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::price_components::MatrixDimension)]
pub struct MatrixDimension {
    pub key: String,
    pub value: String,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::price_components::TierRow)]
pub struct TierRow {
    pub first_unit: u64,
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
    #[schema(value_type = String, format = "decimal")]
    pub flat_fee: Option<rust_decimal::Decimal>,
    #[schema(value_type = String, format = "decimal")]
    pub flat_cap: Option<rust_decimal::Decimal>,
}

#[derive(o2o, Clone, ToSchema, Serialize, Deserialize, Debug)]
#[map_owned(meteroid_store::domain::enums::BillingType)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BillingTypeEnum {
    Advance,
    Arrears,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::price_components::UsagePricingModel)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UsagePricingModel {
    PerUnit {
        #[schema(value_type = String, format = "decimal")]
        rate: rust_decimal::Decimal,
    },
    Tiered {
        #[map(~.into_iter().map(|x| x.into()).collect())]
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Volume {
        #[map(~.into_iter().map(|x| x.into()).collect())]
        tiers: Vec<TierRow>,
        block_size: Option<u64>,
    },
    Package {
        block_size: u64,
        #[schema(value_type = String, format = "decimal")]
        rate: rust_decimal::Decimal,
    },
    Matrix {
        #[map(~.into_iter().map(|x| x.into()).collect())]
        rates: Vec<MatrixRow>,
    },
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[owned_into(meteroid_store::domain::subscription_components::SubscriptionComponentNewInternal)]
#[from_owned(meteroid_store::domain::subscription_components::SubscriptionComponent)]
#[ghosts(is_override: {@.price_component_id.is_some()})]
pub struct SubscriptionComponent {
    #[serde(default, with = "string_serde_opt")]
    pub price_component_id: Option<PriceComponentId>,
    #[serde(default, with = "string_serde_opt")]
    pub product_id: Option<ProductId>,
    pub name: String,
    #[map(~.into())]
    pub period: SubscriptionFeeBillingPeriodEnum,
    #[map(~.into())]
    pub fee: SubscriptionFee,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::subscription_components::ComponentParameterization)]
pub struct ComponentParameterization {
    #[serde(with = "string_serde")]
    pub component_id: PriceComponentId,
    #[map(~.into())]
    pub parameters: ComponentParameters,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::subscription_components::ComponentParameters)]
pub struct ComponentParameters {
    pub initial_slot_count: Option<u32>,
    #[map(~.map(| x | x.into()))]
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[owned_into(meteroid_store::domain::subscription_components::ComponentOverride)]
pub struct ComponentOverride {
    #[serde(with = "string_serde")]
    pub component_id: PriceComponentId,
    #[map(~.into())]
    pub component: SubscriptionComponent,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[owned_into(meteroid_store::domain::subscription_components::ExtraComponent)]
pub struct ExtraComponent {
    #[map(~.into())]
    pub component: SubscriptionComponent,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::subscription_add_ons::SubscriptionAddOnOverride)]
pub struct SubscriptionAddOnOverride {
    pub name: String,
    #[map(~.into())]
    pub period: SubscriptionFeeBillingPeriodEnum,
    #[map(~.into())]
    pub fee: SubscriptionFee,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[from_owned(meteroid_store::domain::subscription_add_ons::SubscriptionAddOn)]
pub struct SubscriptionAddOn {
    #[serde(default, with = "string_serde")]
    pub add_on_id: AddOnId,
    pub name: String,
    #[map(~.into())]
    pub period: SubscriptionFeeBillingPeriodEnum,
    #[map(~.into())]
    pub fee: SubscriptionFee,
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[map_owned(meteroid_store::domain::subscription_add_ons::SubscriptionAddOnParameterization)]
pub struct SubscriptionAddOnParameterization {
    pub initial_slot_count: Option<u32>,
    #[map(~.map(| x | x.into()))]
    pub billing_period: Option<BillingPeriodEnum>,
    pub committed_capacity: Option<u64>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionAddOnCustomization {
    Override(SubscriptionAddOnOverride),
    Parameterization(SubscriptionAddOnParameterization),
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CreateSubscriptionAddOn {
    #[serde(with = "string_serde")]
    pub add_on_id: AddOnId,
    pub customization: Option<SubscriptionAddOnCustomization>,
}

#[derive(ToSchema, Serialize, Deserialize, Validate, Debug)]
pub struct CancelSubscriptionRequest {
    /// If not provided, the cancellation will be effective at the end of the current billing or committed period.
    pub effective_date: Option<NaiveDate>,
    pub reason: Option<String>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct CancelSubscriptionResponse {
    pub subscription: Subscription,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct SubscriptionListResponse {
    pub data: Vec<Subscription>,
    pub pagination_meta: PaginationResponse,
}

// #[derive(ToSchema, Serialize, Deserialize, Validate, Debug)]
// pub struct ChangeSubscriptionPlanRequest {
//     #[serde(with = "string_serde")]
//     pub new_plan_id: PlanId,
//     pub new_plan_version: Option<i32>,
//     pub effective_date: Option<NaiveDate>,
// }
//
// #[derive(ToSchema, Serialize, Deserialize, Debug)]
// pub struct ChangeSubscriptionPlanResponse {
//     pub subscription: Subscription,
//     pub message: String,
// }
