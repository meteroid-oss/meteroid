use crate::currencies::model::Currency;
use crate::entitlements::model::Entitlement;
use chrono::NaiveDate;
use common_domain::ids::{
    AddOnId, AliasOr, AppliedCouponId, BankAccountId, BillableMetricId, CouponId, CreditUnitId,
    CustomerId, PlanVersionId, PriceComponentId, PriceId, ProductId, SubscriptionAddOnId,
};
use common_domain::ids::{PlanId, string_serde_opt, string_serde_vec_opt};
use common_domain::ids::{SubscriptionId, string_serde};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{BillingPeriodEnum, PaginatedRequest, PaginationResponse};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct OnlinePaymentMethodConfig {
    /// If None, inherits all online providers from invoicing entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<OnlineMethodsConfig>,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct BankTransferPaymentMethodConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<BankAccountId>,
}

#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
pub struct ExternalPaymentMethodConfig {}

/// Online (card/direct debit), BankTransfer, or External.
#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethodsConfig {
    Online(OnlinePaymentMethodConfig),
    BankTransfer(BankTransferPaymentMethodConfig),
    External(ExternalPaymentMethodConfig),
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
            PaymentMethodsConfig::Online(c) => {
                meteroid_store::domain::subscriptions::PaymentMethodsConfig::Online {
                    config: c.config.map(|c| c.into()),
                }
            }
            PaymentMethodsConfig::BankTransfer(c) => {
                meteroid_store::domain::subscriptions::PaymentMethodsConfig::BankTransfer {
                    account_id: c.account_id,
                }
            }
            PaymentMethodsConfig::External(_) => {
                meteroid_store::domain::subscriptions::PaymentMethodsConfig::External
            }
        }
    }
}

impl From<meteroid_store::domain::subscriptions::PaymentMethodsConfig> for PaymentMethodsConfig {
    fn from(config: meteroid_store::domain::subscriptions::PaymentMethodsConfig) -> Self {
        match config {
            meteroid_store::domain::subscriptions::PaymentMethodsConfig::Online { config } => {
                PaymentMethodsConfig::Online(OnlinePaymentMethodConfig {
                    config: config.map(|c| c.into()),
                })
            }
            meteroid_store::domain::subscriptions::PaymentMethodsConfig::BankTransfer {
                account_id,
            } => PaymentMethodsConfig::BankTransfer(BankTransferPaymentMethodConfig { account_id }),
            meteroid_store::domain::subscriptions::PaymentMethodsConfig::External => {
                PaymentMethodsConfig::External(ExternalPaymentMethodConfig {})
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
    #[param(inline)]
    pub statuses: Option<Vec<SubscriptionStatusEnum>>,
    /// Sort order. Format: `column.direction`. Allowed columns: `customer_name`, `plan_name`, `mrr_cents`, `billing_start_date`, `end_date`, `status`, `created_at`. Direction: `asc` or `desc`. Default: `created_at.desc`.
    pub order_by: Option<String>,
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
    pub created_at: ApiDateTime,
    /// When the subscription was activated (first payment or activation condition met)
    pub activated_at: Option<ApiDateTime>,
    pub purchase_order: Option<String>,
    /// The subscription's prices are quoted tax-included (snapshotted from its plan version).
    pub tax_inclusive: bool,
    /// If false, invoices will stay in Draft until manually reviewed and finalized. Default to true.
    pub auto_advance_invoices: bool,
    /// Automatically try to charge the customer's configured payment method on finalize.
    pub charge_automatically: bool,
    /// Payment methods configuration (Online, BankTransfer, or External)
    pub payment_methods_config: Option<PaymentMethodsConfig>,
    /// User-defined custom property values, keyed by definition `key`.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
    /// The customer's events this subscription bills, when it does not bill all of them.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage_filter: Option<UsageFilter>,
    /// When the current usage filter, or its removal, started to apply: the start of the usage
    /// neither this subscription nor the customer's others had invoiced when it was set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage_filter_effective_from: Option<ApiDateTime>,
}

/// Restricts the customer's usage events a subscription bills to those whose `property` equals
/// `value` (for example `api_key` = `key-A`). Among the customer's subscriptions pricing the same
/// metric, one with a usage filter bills the events matching it (an event two filters match goes
/// to the older subscription); one without bills the events matching no other subscription's
/// filter. Two subscriptions without a usage filter both bill every such event. Events matching
/// no filter while every subscription pricing their metric has one are billed by none and
/// reported as unattributed usage.
#[derive(Clone, ToSchema, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct UsageFilter {
    /// Event property name, e.g. `api_key`.
    #[schema(examples("api_key"))]
    pub property: String,
    /// Value the property must have.
    #[schema(examples("key-A"))]
    pub value: String,
}

/// Coupon as embedded in subscription details — a subset of the `Coupon` resource
/// returned by the coupons API.
#[derive(Clone, ToSchema, Serialize, Deserialize, Debug)]
#[schema(as = SubscriptionCoupon)]
pub struct Coupon {
    #[serde(with = "string_serde")]
    pub id: CouponId,
    pub code: String,
    pub description: String,
    pub discount: crate::coupons::model::CouponDiscount,
    pub expires_at: Option<ApiDateTime>,
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
    pub last_applied_at: Option<ApiDateTime>,
    pub created_at: ApiDateTime,
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
    pub created_at: ApiDateTime,
    /// When the subscription was activated (first payment or activation condition met)
    pub activated_at: Option<ApiDateTime>,
    pub purchase_order: Option<String>,
    /// The subscription's prices are quoted tax-included (snapshotted from its plan version).
    pub tax_inclusive: bool,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub payment_methods_config: Option<PaymentMethodsConfig>,
    pub components: Vec<SubscriptionComponent>,
    pub add_ons: Vec<SubscriptionAddOn>,
    pub applied_coupons: Vec<AppliedCouponDetailed>,
    pub checkout_url: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub entitlements: Vec<Entitlement>,
    /// Materialized minimum-commitment floor on this subscription (plan snapshot or operator
    /// override). Read-only here — set it at creation, or change it via a subscription amendment.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub minimum_commitment: Option<crate::plans::model::MinimumCommitment>,
    /// User-defined custom property values, keyed by definition `key`.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
    /// The customer's events this subscription bills, when it does not bill all of them.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage_filter: Option<UsageFilter>,
    /// When the current usage filter, or its removal, started to apply: the start of the usage
    /// neither this subscription nor the customer's others had invoiced when it was set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage_filter_effective_from: Option<ApiDateTime>,
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

    /// Codes of the coupons to apply. A code matches whatever the case of its ASCII letters; when
    /// several coupons differ from it only in that case, the one with exactly that code applies. A
    /// code that names no live coupon, or several differing only in case, refuses the request with
    /// 400 listing the codes (unknown codes used to be ignored silently).
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
    /// Historical import mode: when true, invoices finalized for this subscription keep their
    /// billing-period date as the invoice date instead of being stamped with the emission date.
    #[schema(nullable = false)]
    pub backdate_invoices: Option<bool>,
    /// User-defined custom property values, keyed by definition `key`. Validated against the
    /// tenant's subscription definitions.
    #[schema(nullable = false, value_type = Option<serde_json::Value>)]
    pub custom_properties: Option<serde_json::Value>,
    /// Bill only the customer's events whose property has this value. Omit to bill the events
    /// no usage filter of the customer's other subscriptions matches.
    #[schema(nullable = false)]
    pub usage_filter: Option<UsageFilter>,
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
#[schema(title = "SubscriptionStatus")]
pub enum SubscriptionStatusEnum {
    PendingActivation,
    PendingCharge,
    TrialActive,
    Active,
    TrialExpired,
    Paused,
    Suspended,
    Cancelled,
    Aborted,
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

/// A credit pack: charged like a fixed fee, it grants `credit_amount` × `quantity` of credit once
/// paid, in the subscription currency or in `credit_unit`. A zero `unit_price` makes it credit
/// included with the plan (promotional). A currency pack priced below its credit grants the price
/// paid as paid credit and the difference as promotional bonus credit, which applies before tax.
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CreditFee {
    /// Price of one pack, in major units of the subscription currency.
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: rust_decimal::Decimal,
    /// Packs billed and granted each time.
    pub quantity: u32,
    /// Credit granted per pack, in major units of the subscription currency, or a number of
    /// credits of `credit_unit` when set.
    #[schema(value_type = String, format = "decimal")]
    pub credit_amount: rust_decimal::Decimal,
    /// How long the granted credit stays spendable; absent never expires. DAY and MONTH run from
    /// payment; BILLING_PERIOD ends with the invoiced period.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<crate::plans::model::CreditPackExpiry>,
    /// When true, billed and granted once, on the subscription's first invoice, instead of every
    /// period.
    #[serde(default)]
    pub once: bool,
    /// The credit unit the pack grants; absent grants the subscription currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditPackUnit>,
    /// Included packs only: unspent credit carried once into a new grant when it expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollover: Option<crate::plans::model::CreditPackRollover>,
}

/// The credit unit a credit pack grants.
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CreditPackUnit {
    /// The credit unit's id (`cun_…`).
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub credit_unit_id: CreditUnitId,
    /// The credit unit's code, e.g. `CREDITS`.
    pub code: String,
    /// Decimal places of the unit: credit balances and ledger amounts in it are integers of
    /// 10^-precision credit.
    pub precision: u8,
}

impl From<meteroid_store::domain::credit_units::CreditUnitRef> for CreditPackUnit {
    fn from(unit: meteroid_store::domain::credit_units::CreditUnitRef) -> Self {
        Self {
            credit_unit_id: unit.unit_id,
            code: unit.code,
            precision: unit.precision,
        }
    }
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
    /// Set when the overage is priced in a credit unit: `overage_rate` is then a number of
    /// credits, drawn from the customer's balance in that unit before anything is billed in
    /// money. Taken from the plan; read-only here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(read_only)]
    pub credit_unit: Option<FeeCreditUnit>,
}

/// The credit unit a usage fee's amounts are expressed in, with its value in the subscription
/// currency.
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct FeeCreditUnit {
    /// The credit unit's id (`cun_…`).
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub credit_unit_id: CreditUnitId,
    /// The credit unit's code, e.g. `CREDITS`.
    pub code: String,
    /// Decimal places of the unit: credit balances and ledger amounts in it are integers of
    /// 10^-precision credit.
    pub precision: u8,
    /// Subscription-currency major units per credit: the money billed, before tax, for each
    /// credit the customer's balance in the unit does not cover.
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
}

impl From<meteroid_store::domain::credit_units::FeeCreditUnit> for FeeCreditUnit {
    fn from(unit: meteroid_store::domain::credit_units::FeeCreditUnit) -> Self {
        Self {
            credit_unit_id: unit.unit_id,
            code: unit.code,
            precision: unit.precision,
            rate: unit.rate,
        }
    }
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
    /// Set when the usage is priced in a credit unit: the model's amounts are then numbers of
    /// credits, drawn from the customer's balance in that unit before anything is billed in
    /// money. Taken from the plan; read-only here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(read_only)]
    pub credit_unit: Option<FeeCreditUnit>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
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
    #[serde(rename = "CREDIT")]
    Credit(CreditFee),
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
                credit_unit,
            } => SubscriptionFee::Capacity(CapacityFee {
                rate,
                included,
                overage_rate,
                metric_id,
                credit_unit: credit_unit.map(Into::into),
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
            DomainFee::Usage {
                metric_id,
                model,
                credit_unit,
            } => SubscriptionFee::Usage(UsageFee {
                metric_id,
                model: model.into(),
                credit_unit: credit_unit.map(Into::into),
            }),
            DomainFee::Credit {
                rate,
                quantity,
                credit_amount,
                expiry,
                once,
                credit_unit,
                rollover,
            } => SubscriptionFee::Credit(CreditFee {
                unit_price: rate,
                quantity,
                credit_amount,
                expiry: crate::plans::mapping::credit_expiry_to_rest(expiry),
                once,
                credit_unit: credit_unit.map(Into::into),
                rollover: rollover
                    .as_ref()
                    .map(crate::plans::mapping::credit_rollover_to_rest),
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
                credit_unit: None,
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
                credit_unit: None,
            },
            // The store refuses an unusable pack (zero expiry, fractional credit) on creation.
            SubscriptionFee::Credit(f) => DomainFee::Credit {
                rate: f.unit_price,
                quantity: f.quantity,
                credit_amount: f.credit_amount,
                expiry: f.expiry.map(|expiry| {
                    use crate::plans::model::CreditExpiryUnit;
                    use meteroid_store::domain::price_components::CreditExpiry;
                    match expiry.unit {
                        CreditExpiryUnit::Day => CreditExpiry::Day(expiry.value),
                        CreditExpiryUnit::Month => CreditExpiry::Month(expiry.value),
                        CreditExpiryUnit::BillingPeriod => CreditExpiry::BillingPeriod,
                    }
                }),
                credit_unit: None,
                rollover: None,
                once: f.once,
            },
        }
    }
}

// Pricing row types are shared with the plans API — re-exported so both surfaces emit a single
// `TierRow` / `MatrixRow` / `MatrixDimension` OpenAPI schema instead of colliding definitions.
pub use crate::plans::model::{MatrixRow, TierRow};

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
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
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
    effective_from: {common_domain::clock::now().naive_utc().date()},
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

// ── PriceEntry / ProductRef types ───────────────────────────────

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PriceEntry {
    #[serde(rename = "EXISTING")]
    Existing(ExistingPriceRef),
    #[serde(rename = "NEW")]
    New(PriceInput),
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ExistingPriceRef {
    #[serde(with = "string_serde")]
    pub id: PriceId,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct PriceInput {
    pub cadence: BillingPeriodEnum,
    pub currency: String,
    pub pricing: Pricing,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Pricing {
    #[serde(rename = "RATE")]
    Rate(RatePricing),
    #[serde(rename = "SLOT")]
    Slot(SlotPricing),
    #[serde(rename = "CAPACITY")]
    Capacity(CapacityPricing),
    #[serde(rename = "USAGE")]
    Usage(UsagePricing),
    #[serde(rename = "EXTRA_RECURRING")]
    ExtraRecurring(ExtraRecurringPricing),
    #[serde(rename = "ONE_TIME")]
    OneTime(OneTimePricing),
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct RatePricing {
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct SlotPricing {
    #[schema(value_type = String, format = "decimal")]
    pub unit_rate: rust_decimal::Decimal,
    pub min_slots: Option<u32>,
    pub max_slots: Option<u32>,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct CapacityPricing {
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
    pub included: u64,
    #[schema(value_type = String, format = "decimal")]
    pub overage_rate: rust_decimal::Decimal,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct UsagePricing {
    pub model: UsagePricingModel,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ExtraRecurringPricing {
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: rust_decimal::Decimal,
    pub quantity: u32,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct OneTimePricing {
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: rust_decimal::Decimal,
    pub quantity: u32,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProductRef {
    #[serde(rename = "EXISTING")]
    Existing(ExistingProductRef),
    #[serde(rename = "NEW")]
    New(NewProductRef),
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ExistingProductRef {
    #[serde(with = "string_serde")]
    pub id: ProductId,
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct NewProductRef {
    pub name: String,
    pub fee_type: crate::products::model::ProductFeeTypeEnum,
    pub fee_structure: crate::products::model::ProductFeeStructure,
}

// ── PriceEntry / ProductRef From conversions ───────────────────

impl From<PriceEntry> for meteroid_store::domain::price_components::PriceEntry {
    fn from(val: PriceEntry) -> Self {
        match val {
            PriceEntry::Existing(e) => {
                meteroid_store::domain::price_components::PriceEntry::Existing(e.id)
            }
            PriceEntry::New(input) => {
                meteroid_store::domain::price_components::PriceEntry::New(input.into())
            }
        }
    }
}

impl From<PriceInput> for meteroid_store::domain::price_components::PriceInput {
    fn from(val: PriceInput) -> Self {
        Self {
            cadence: val.cadence.into(),
            currency: val.currency,
            pricing: val.pricing.into(),
            credit_unit: None,
        }
    }
}

impl From<Pricing> for meteroid_store::domain::prices::Pricing {
    fn from(val: Pricing) -> Self {
        match val {
            Pricing::Rate(r) => meteroid_store::domain::prices::Pricing::Rate { rate: r.rate },
            Pricing::Slot(s) => meteroid_store::domain::prices::Pricing::Slot {
                unit_rate: s.unit_rate,
                min_slots: s.min_slots,
                max_slots: s.max_slots,
            },
            Pricing::Capacity(c) => meteroid_store::domain::prices::Pricing::Capacity {
                rate: c.rate,
                included: c.included,
                overage_rate: c.overage_rate,
            },
            Pricing::Usage(u) => meteroid_store::domain::prices::Pricing::Usage(u.model.into()),
            Pricing::ExtraRecurring(e) => meteroid_store::domain::prices::Pricing::ExtraRecurring {
                unit_price: e.unit_price,
                quantity: e.quantity,
            },
            Pricing::OneTime(o) => meteroid_store::domain::prices::Pricing::OneTime {
                unit_price: o.unit_price,
                quantity: o.quantity,
            },
        }
    }
}

impl From<ProductRef> for meteroid_store::domain::price_components::ProductRef {
    fn from(val: ProductRef) -> Self {
        match val {
            ProductRef::Existing(e) => {
                meteroid_store::domain::price_components::ProductRef::Existing(e.id)
            }
            ProductRef::New(n) => meteroid_store::domain::price_components::ProductRef::New {
                name: n.name,
                fee_type: n.fee_type.into(),
                fee_structure: n.fee_structure.into(),
            },
        }
    }
}

// ── End PriceEntry / ProductRef ────────────────────────────────

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ComponentOverride {
    #[serde(with = "string_serde")]
    pub component_id: PriceComponentId,
    pub name: String,
    pub price_entry: PriceEntry,
}

impl From<ComponentOverride>
    for meteroid_store::domain::subscription_components::ComponentOverride
{
    fn from(val: ComponentOverride) -> Self {
        Self {
            component_id: val.component_id,
            name: val.name,
            price_entry: val.price_entry.into(),
        }
    }
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct ExtraComponent {
    pub name: String,
    pub product_ref: ProductRef,
    pub price_entry: PriceEntry,
}

impl From<ExtraComponent> for meteroid_store::domain::subscription_components::ExtraComponent {
    fn from(val: ExtraComponent) -> Self {
        Self {
            name: val.name,
            product_ref: val.product_ref.into(),
            price_entry: val.price_entry.into(),
        }
    }
}

#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct SubscriptionAddOnPriceOverride {
    pub name: Option<String>,
    pub price_entry: PriceEntry,
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
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
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
    /// Partial update of custom property values (merge; send a key with `null` to remove it).
    /// Validated against the tenant's `SUBSCRIPTION` property definitions. Omit to leave unchanged.
    #[serde(default)]
    #[schema(value_type = Option<serde_json::Value>)]
    pub custom_properties: Option<serde_json::Value>,
    /// Omit to leave the usage filter unchanged; `null` removes it; an object replaces it. The
    /// change applies to all usage not yet invoiced, the current period included.
    #[serde(
        default,
        with = "::serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<UsageFilter>)]
    pub usage_filter: Option<Option<UsageFilter>>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct SubscriptionUpdateResponse {
    pub subscription: SubscriptionDetails,
}
