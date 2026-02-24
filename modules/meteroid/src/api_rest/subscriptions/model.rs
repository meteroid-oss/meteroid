use crate::api_rest::currencies::model::Currency;
use crate::api_rest::model::{BillingPeriodEnum, PaginatedRequest, PaginationResponse};
use chrono::NaiveDate;
use common_domain::ids::{
    AddOnId, AliasOr, AppliedCouponId, BankAccountId, BillableMetricId, CouponId, CustomerId,
    PlanVersionId, PriceComponentId, ProductId, SubscriptionAddOnId,
};
use common_domain::ids::{PlanId, string_serde_opt, string_serde_vec_opt};
use common_domain::ids::{SubscriptionId, string_serde};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

/// Online (card/direct debit), BankTransfer, or External.
#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethodsConfig {
    Online {
        /// If None, inherits all online providers from invoicing entity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<OnlineMethodsConfig>,
    },
    BankTransfer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schema(value_type = Option<String>)]
        account_id: Option<BankAccountId>,
    },
    External,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug, Default)]
pub struct OnlineMethodsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<OnlineMethodConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_debit: Option<OnlineMethodConfig>,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct OnlineMethodConfig {
    pub enabled: bool,
}

impl From<PaymentMethodsConfig> for meteroid_store::domain::subscriptions::PaymentMethodsConfig {
    fn from(config: PaymentMethodsConfig) -> Self {
        match config {
            PaymentMethodsConfig::Online { config } => {
                meteroid_store::domain::subscriptions::PaymentMethodsConfig::Online {
                    config: config.map(|c| c.into()),
                }
            }
            PaymentMethodsConfig::BankTransfer { account_id } => {
                meteroid_store::domain::subscriptions::PaymentMethodsConfig::BankTransfer {
                    account_id,
                }
            }
            PaymentMethodsConfig::External => {
                meteroid_store::domain::subscriptions::PaymentMethodsConfig::External
            }
        }
    }
}

impl From<meteroid_store::domain::subscriptions::PaymentMethodsConfig> for PaymentMethodsConfig {
    fn from(config: meteroid_store::domain::subscriptions::PaymentMethodsConfig) -> Self {
        match config {
            meteroid_store::domain::subscriptions::PaymentMethodsConfig::Online { config } => {
                PaymentMethodsConfig::Online {
                    config: config.map(|c| c.into()),
                }
            }
            meteroid_store::domain::subscriptions::PaymentMethodsConfig::BankTransfer {
                account_id,
            } => PaymentMethodsConfig::BankTransfer { account_id },
            meteroid_store::domain::subscriptions::PaymentMethodsConfig::External => {
                PaymentMethodsConfig::External
            }
        }
    }
}

impl From<OnlineMethodsConfig> for meteroid_store::domain::subscriptions::OnlineMethodsConfig {
    fn from(config: OnlineMethodsConfig) -> Self {
        meteroid_store::domain::subscriptions::OnlineMethodsConfig {
            card: config.card.map(|c| c.into()),
            direct_debit: config.direct_debit.map(|c| c.into()),
        }
    }
}

impl From<meteroid_store::domain::subscriptions::OnlineMethodsConfig> for OnlineMethodsConfig {
    fn from(config: meteroid_store::domain::subscriptions::OnlineMethodsConfig) -> Self {
        OnlineMethodsConfig {
            card: config.card.map(|c| c.into()),
            direct_debit: config.direct_debit.map(|c| c.into()),
        }
    }
}

impl From<OnlineMethodConfig> for meteroid_store::domain::subscriptions::OnlineMethodConfig {
    fn from(config: OnlineMethodConfig) -> Self {
        meteroid_store::domain::subscriptions::OnlineMethodConfig {
            enabled: config.enabled,
        }
    }
}

impl From<meteroid_store::domain::subscriptions::OnlineMethodConfig> for OnlineMethodConfig {
    fn from(config: meteroid_store::domain::subscriptions::OnlineMethodConfig) -> Self {
        OnlineMethodConfig {
            enabled: config.enabled,
        }
    }
}

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct SubscriptionRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Filter by customer ID or alias
    #[param(value_type = String, required = false)]
    pub customer_id: Option<AliasOr<CustomerId>>,
    #[serde(default, with = "string_serde_opt")]
    pub plan_id: Option<PlanId>,
    pub statuses: Option<Vec<SubscriptionStatusEnum>>,
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
    pub plan_description: Option<String>,
    #[serde(with = "string_serde")]
    pub plan_version_id: PlanVersionId,
    pub plan_version: u32,
    pub status: SubscriptionStatusEnum,
    /// When the subscription contract starts (benefits apply from this date)
    pub start_date: NaiveDate,
    /// When the subscription ends (if set)
    pub end_date: Option<NaiveDate>,
    /// When billing started (after any trial period)
    pub billing_start_date: Option<NaiveDate>,
    /// Current billing period start date
    pub current_period_start: NaiveDate,
    /// Current billing period end date
    pub current_period_end: Option<NaiveDate>,
    /// Trial duration in days
    pub trial_duration: Option<u32>,
    /// Payment terms in days (0 = due on issue)
    pub net_terms: u32,
    /// Default memo for invoices
    pub invoice_memo: Option<String>,
    /// Monthly recurring revenue in cents
    pub mrr_cents: u64,
    /// Billing period (monthly, annual, etc.)
    pub period: BillingPeriodEnum,
    /// When the subscription was created
    pub created_at: chrono::NaiveDateTime,
    /// When the subscription was activated (first payment or activation condition met)
    pub activated_at: Option<chrono::NaiveDateTime>,
    pub purchase_order: Option<String>,
    /// If false, invoices will stay in Draft until manually reviewed and finalized. Default to true.
    pub auto_advance_invoices: bool,
    /// Automatically try to charge the customer's configured payment method on finalize.
    pub charge_automatically: bool,
    /// Payment methods configuration (Online, BankTransfer, or External)
    pub payment_methods_config: Option<PaymentMethodsConfig>,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct PercentageDiscount {
    #[schema(value_type = String, format = "decimal")]
    pub percentage: rust_decimal::Decimal,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct FixedDiscount {
    pub currency: String,
    #[schema(value_type = String, format = "decimal")]
    pub amount: rust_decimal::Decimal,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CouponDiscount {
    #[serde(rename = "PERCENTAGE")]
    Percentage(PercentageDiscount),
    #[serde(rename = "FIXED")]
    Fixed(FixedDiscount),
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
    /// When the subscription contract starts (benefits apply from this date)
    pub start_date: NaiveDate,
    /// When the subscription ends (if set)
    pub end_date: Option<NaiveDate>,
    /// When billing started (after any trial period)
    pub billing_start_date: Option<NaiveDate>,
    /// Current billing period start date
    pub current_period_start: NaiveDate,
    /// Current billing period end date
    pub current_period_end: Option<NaiveDate>,
    /// Trial duration in days
    pub trial_duration: Option<u32>,
    /// Payment terms in days (0 = due on issue)
    pub net_terms: u32,
    /// Default memo for invoices
    pub invoice_memo: Option<String>,
    /// Monthly recurring revenue in cents
    pub mrr_cents: u64,
    /// Billing period (monthly, annual, etc.)
    pub period: BillingPeriodEnum,
    /// When the subscription was created
    pub created_at: chrono::NaiveDateTime,
    /// When the subscription was activated (first payment or activation condition met)
    pub activated_at: Option<chrono::NaiveDateTime>,
    pub purchase_order: Option<String>,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub payment_methods_config: Option<PaymentMethodsConfig>,
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
    #[schema(examples("2024-11-01"))]
    pub start_date: NaiveDate,
    #[schema(examples("2025-11-01"))]
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
    /// Payment methods configuration. If not specified, inherits from the invoicing entity.
    #[schema(nullable = false)]
    pub payment_methods_config: Option<PaymentMethodsConfig>,
    /// Migration mode: when true with a past start_date, skip creating invoices for past cycles.
    /// The subscription will be set to the current billing period with correct cycle_index.
    #[schema(nullable = false)]
    pub skip_past_invoices: Option<bool>,
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
    Errored,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CreateSubscriptionComponents {
    pub parameterized_components: Option<Vec<ComponentParameterization>>,
    pub overridden_components: Option<Vec<ComponentOverride>>,
    pub extra_components: Option<Vec<ExtraComponent>>,
    #[serde(default, with = "string_serde_vec_opt")]
    pub remove_components: Option<Vec<PriceComponentId>>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct RateFee {
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct OneTimeFee {
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
    pub quantity: u32,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct RecurringFee {
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
    pub quantity: u32,
    pub billing_type: BillingTypeEnum,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CapacityFee {
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
    pub included: u64,
    #[schema(value_type = String, format = "decimal")]
    pub overage_rate: rust_decimal::Decimal,
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct SlotFee {
    pub unit: String,
    #[schema(value_type = String, format = "decimal")]
    pub unit_rate: rust_decimal::Decimal,
    pub min_slots: Option<u32>,
    pub max_slots: Option<u32>,
    pub initial_slots: u32,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct UsageFee {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub model: UsagePricingModel,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SubscriptionFee {
    #[serde(rename = "RATE")]
    Rate(RateFee),
    #[serde(rename = "ONE_TIME")]
    OneTime(OneTimeFee),
    #[serde(rename = "RECURRING")]
    Recurring(RecurringFee),
    #[serde(rename = "CAPACITY")]
    Capacity(CapacityFee),
    #[serde(rename = "SLOT")]
    Slot(SlotFee),
    #[serde(rename = "USAGE")]
    Usage(UsageFee),
}

impl From<meteroid_store::domain::subscription_components::SubscriptionFee> for SubscriptionFee {
    fn from(fee: meteroid_store::domain::subscription_components::SubscriptionFee) -> Self {
        use meteroid_store::domain::subscription_components::SubscriptionFee as DomainFee;
        match fee {
            DomainFee::Rate { rate } => SubscriptionFee::Rate(RateFee { rate }),
            DomainFee::OneTime { rate, quantity } => {
                SubscriptionFee::OneTime(OneTimeFee { rate, quantity })
            }
            DomainFee::Recurring {
                rate,
                quantity,
                billing_type,
            } => SubscriptionFee::Recurring(RecurringFee {
                rate,
                quantity,
                billing_type: billing_type.into(),
            }),
            DomainFee::Capacity {
                rate,
                included,
                overage_rate,
                metric_id,
            } => SubscriptionFee::Capacity(CapacityFee {
                rate,
                included,
                overage_rate,
                metric_id,
            }),
            DomainFee::Slot {
                unit,
                unit_rate,
                min_slots,
                max_slots,
                initial_slots,
            } => SubscriptionFee::Slot(SlotFee {
                unit,
                unit_rate,
                min_slots,
                max_slots,
                initial_slots,
            }),
            DomainFee::Usage { metric_id, model } => SubscriptionFee::Usage(UsageFee {
                metric_id,
                model: model.into(),
            }),
        }
    }
}

impl From<SubscriptionFee> for meteroid_store::domain::subscription_components::SubscriptionFee {
    fn from(fee: SubscriptionFee) -> Self {
        use meteroid_store::domain::subscription_components::SubscriptionFee as DomainFee;
        match fee {
            SubscriptionFee::Rate(f) => DomainFee::Rate { rate: f.rate },
            SubscriptionFee::OneTime(f) => DomainFee::OneTime {
                rate: f.rate,
                quantity: f.quantity,
            },
            SubscriptionFee::Recurring(f) => DomainFee::Recurring {
                rate: f.rate,
                quantity: f.quantity,
                billing_type: f.billing_type.into(),
            },
            SubscriptionFee::Capacity(f) => DomainFee::Capacity {
                rate: f.rate,
                included: f.included,
                overage_rate: f.overage_rate,
                metric_id: f.metric_id,
            },
            SubscriptionFee::Slot(f) => DomainFee::Slot {
                unit: f.unit,
                unit_rate: f.unit_rate,
                min_slots: f.min_slots,
                max_slots: f.max_slots,
                initial_slots: f.initial_slots,
            },
            SubscriptionFee::Usage(f) => DomainFee::Usage {
                metric_id: f.metric_id,
                model: f.model.into(),
            },
        }
    }
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

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct PerUnitPricing {
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct TieredPricing {
    pub tiers: Vec<TierRow>,
    pub block_size: Option<u64>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct VolumePricing {
    pub tiers: Vec<TierRow>,
    pub block_size: Option<u64>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct PackagePricing {
    pub block_size: u64,
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct MatrixPricing {
    pub rates: Vec<MatrixRow>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UsagePricingModel {
    #[serde(rename = "PER_UNIT")]
    PerUnit(PerUnitPricing),
    #[serde(rename = "TIERED")]
    Tiered(TieredPricing),
    #[serde(rename = "VOLUME")]
    Volume(VolumePricing),
    #[serde(rename = "PACKAGE")]
    Package(PackagePricing),
    #[serde(rename = "MATRIX")]
    Matrix(MatrixPricing),
}

impl From<meteroid_store::domain::price_components::UsagePricingModel> for UsagePricingModel {
    fn from(model: meteroid_store::domain::price_components::UsagePricingModel) -> Self {
        use meteroid_store::domain::price_components::UsagePricingModel as DomainModel;
        match model {
            DomainModel::PerUnit { rate } => UsagePricingModel::PerUnit(PerUnitPricing { rate }),
            DomainModel::Tiered { tiers, block_size } => UsagePricingModel::Tiered(TieredPricing {
                tiers: tiers.into_iter().map(|t| t.into()).collect(),
                block_size,
            }),
            DomainModel::Volume { tiers, block_size } => UsagePricingModel::Volume(VolumePricing {
                tiers: tiers.into_iter().map(|t| t.into()).collect(),
                block_size,
            }),
            DomainModel::Package { block_size, rate } => {
                UsagePricingModel::Package(PackagePricing { block_size, rate })
            }
            DomainModel::Matrix { rates } => UsagePricingModel::Matrix(MatrixPricing {
                rates: rates.into_iter().map(|r| r.into()).collect(),
            }),
        }
    }
}

impl From<UsagePricingModel> for meteroid_store::domain::price_components::UsagePricingModel {
    fn from(model: UsagePricingModel) -> Self {
        use meteroid_store::domain::price_components::UsagePricingModel as DomainModel;
        match model {
            UsagePricingModel::PerUnit(p) => DomainModel::PerUnit { rate: p.rate },
            UsagePricingModel::Tiered(p) => DomainModel::Tiered {
                tiers: p.tiers.into_iter().map(|t| t.into()).collect(),
                block_size: p.block_size,
            },
            UsagePricingModel::Volume(p) => DomainModel::Volume {
                tiers: p.tiers.into_iter().map(|t| t.into()).collect(),
                block_size: p.block_size,
            },
            UsagePricingModel::Package(p) => DomainModel::Package {
                block_size: p.block_size,
                rate: p.rate,
            },
            UsagePricingModel::Matrix(p) => DomainModel::Matrix {
                rates: p.rates.into_iter().map(|r| r.into()).collect(),
            },
        }
    }
}

#[derive(o2o, ToSchema, Serialize, Deserialize, Clone, Debug)]
#[owned_into(meteroid_store::domain::subscription_components::SubscriptionComponentNewInternal)]
#[from_owned(meteroid_store::domain::subscription_components::SubscriptionComponent)]
#[ghosts(
    is_override: {@.price_component_id.is_some()},
    price_id: {None},
)]
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

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ComponentOverride {
    #[serde(with = "string_serde")]
    pub component_id: PriceComponentId,
    pub name: String,
    #[schema(value_type = Object)]
    pub price_entry: meteroid_store::domain::price_components::PriceEntry,
}

impl From<ComponentOverride>
    for meteroid_store::domain::subscription_components::ComponentOverride
{
    fn from(val: ComponentOverride) -> Self {
        Self {
            component_id: val.component_id,
            name: val.name,
            price_entry: val.price_entry,
        }
    }
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ExtraComponent {
    pub name: String,
    #[schema(value_type = Object)]
    pub product_ref: meteroid_store::domain::price_components::ProductRef,
    #[schema(value_type = Object)]
    pub price_entry: meteroid_store::domain::price_components::PriceEntry,
}

impl From<ExtraComponent> for meteroid_store::domain::subscription_components::ExtraComponent {
    fn from(val: ExtraComponent) -> Self {
        Self {
            name: val.name,
            product_ref: val.product_ref,
            price_entry: val.price_entry,
        }
    }
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionAddOnPriceOverride {
    pub name: Option<String>,
    #[schema(value_type = Object)]
    pub price_entry: meteroid_store::domain::price_components::PriceEntry,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionAddOn {
    #[serde(default, with = "string_serde")]
    pub id: SubscriptionAddOnId,
    #[serde(default, with = "string_serde")]
    pub add_on_id: AddOnId,
    pub name: String,
    pub period: SubscriptionFeeBillingPeriodEnum,
    pub fee: SubscriptionFee,
    pub quantity: u32,
}

impl From<meteroid_store::domain::subscription_add_ons::SubscriptionAddOn> for SubscriptionAddOn {
    fn from(val: meteroid_store::domain::subscription_add_ons::SubscriptionAddOn) -> Self {
        SubscriptionAddOn {
            id: val.id,
            add_on_id: val.add_on_id,
            name: val.name,
            period: val.period.into(),
            fee: val.fee.into(),
            quantity: val.quantity.max(0) as u32,
        }
    }
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
    PriceOverride(SubscriptionAddOnPriceOverride),
    Parameterization(SubscriptionAddOnParameterization),
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CreateSubscriptionAddOn {
    #[serde(with = "string_serde")]
    pub add_on_id: AddOnId,
    pub customization: Option<SubscriptionAddOnCustomization>,
    #[serde(default = "default_quantity")]
    pub quantity: u32,
}

fn default_quantity() -> u32 {
    1
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

#[derive(ToSchema, Serialize, Deserialize, Validate, Debug)]
pub struct SubscriptionUpdateRequest {
    /// Automatically try to charge the customer's configured payment method on finalize.
    pub charge_automatically: Option<bool>,
    /// If false, invoices will stay in Draft until manually reviewed and finalized.
    pub auto_advance_invoices: Option<bool>,
    /// Payment terms in days (0 = due on issue)
    pub net_terms: Option<u32>,
    /// Default memo for invoices
    pub invoice_memo: Option<String>,
    /// Purchase order number
    pub purchase_order: Option<String>,
    /// Payment methods configuration (Online, BankTransfer, or External)
    pub payment_methods_config: Option<PaymentMethodsConfig>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct SubscriptionUpdateResponse {
    pub subscription: SubscriptionDetails,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct SubscriptionUsageResponse {
    #[schema(value_type = String)]
    #[serde(with = "string_serde")]
    pub subscription_id: SubscriptionId,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub metrics: Vec<MetricUsageSummary>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct MetricUsageSummary {
    #[schema(value_type = String)]
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub metric_name: String,
    pub metric_code: String,
    pub total_value: rust_decimal::Decimal,
    pub data_points: Vec<UsageDataPointRest>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct UsageDataPointRest {
    pub window_start: NaiveDate,
    pub window_end: NaiveDate,
    pub value: rust_decimal::Decimal,
    pub dimensions: std::collections::HashMap<String, String>,
}
