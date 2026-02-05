use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::BankAccountId;
use serde::{Deserialize, Serialize};

use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionActivationCondition};
use crate::domain::subscription_add_ons::{
    CreateSubscriptionAddOns, SubscriptionAddOn, SubscriptionAddOnNewInternal,
};
use crate::domain::subscription_components::SubscriptionComponentNewInternal;
use crate::domain::{
    AppliedCouponDetailed, BillableMetric, CreateSubscriptionComponents, CreateSubscriptionCoupons,
    Customer, InvoicingEntity, PlanForSubscription, Schedule, SubscriptionComponent,
    SubscriptionStatusEnum,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::services::PaymentSetupResult;
use common_domain::ids::CouponId;
use common_domain::ids::{
    CustomerId, InvoicingEntityId, PlanId, PlanVersionId, QuoteId, SubscriptionId, TenantId,
};
use diesel_models::enums::CycleActionEnum;
use diesel_models::subscriptions::SubscriptionRowNew;
use diesel_models::subscriptions::{SubscriptionForDisplayRow, SubscriptionRow};
use o2o::o2o;
use uuid::Uuid;

/// Three mutually exclusive payment strategies:
/// - `Online`: Card and/or direct debit
/// - `BankTransfer`: Bank transfer only
/// - `External`: No system payment collection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethodsConfig {
    /// Card and/or direct debit.
    Online {
        /// If None, inherits all online providers from invoicing entity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config: Option<OnlineMethodsConfig>,
    },

    BankTransfer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<BankAccountId>,
    },

    External,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OnlineMethodsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<OnlineMethodConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_debit: Option<OnlineMethodConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnlineMethodConfig {
    pub enabled: bool,
}

impl PaymentMethodsConfig {
    pub fn online() -> Self {
        Self::Online { config: None }
    }

    pub fn online_specific(card: bool, direct_debit: bool) -> Self {
        Self::Online {
            config: Some(OnlineMethodsConfig {
                card: Some(OnlineMethodConfig { enabled: card }),
                direct_debit: Some(OnlineMethodConfig {
                    enabled: direct_debit,
                }),
            }),
        }
    }

    pub fn bank_transfer() -> Self {
        Self::BankTransfer { account_id: None }
    }

    pub fn external() -> Self {
        Self::External
    }

    pub fn is_online(&self) -> bool {
        matches!(self, Self::Online { .. })
    }

    pub fn is_bank_transfer(&self) -> bool {
        matches!(self, Self::BankTransfer { .. })
    }

    pub fn is_external(&self) -> bool {
        matches!(self, Self::External)
    }

    /// For Online with no config (inherit), defaults to true. For non-Online, returns false.
    pub fn card_enabled(&self) -> bool {
        match self {
            Self::Online { config: None } => true,
            Self::Online { config: Some(c) } => c.card.as_ref().map(|m| m.enabled).unwrap_or(true),
            _ => false,
        }
    }

    /// For Online with no config (inherit), defaults to true. For non-Online, returns false.
    pub fn direct_debit_enabled(&self) -> bool {
        match self {
            Self::Online { config: None } => true,
            Self::Online { config: Some(c) } => {
                c.direct_debit.as_ref().map(|m| m.enabled).unwrap_or(true)
            }
            _ => false,
        }
    }

    pub fn bank_transfer_enabled(&self) -> bool {
        matches!(self, Self::BankTransfer { .. })
    }

    pub fn has_online_payment(&self) -> bool {
        self.is_online() && (self.card_enabled() || self.direct_debit_enabled())
    }
}

#[derive(Debug, Clone, o2o)]
#[from_owned(SubscriptionRow)]
pub struct CreatedSubscription {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub billing_day_anchor: i16,
    pub tenant_id: TenantId,
    pub currency: String,
    pub trial_duration: Option<i32>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub billing_start_date: Option<NaiveDate>,
    pub plan_version_id: PlanVersionId,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub mrr_cents: i64,
    #[from(~.into())]
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
    #[ghost({None})]
    pub checkout_url: Option<String>,
    #[from(~.into())]
    pub status: SubscriptionStatusEnum,
    pub purchase_order: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Subscription {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub customer_alias: Option<String>,
    pub customer_name: String,
    pub billing_day_anchor: u16,
    pub tenant_id: TenantId,
    pub currency: String,
    pub trial_duration: Option<u32>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub billing_start_date: Option<NaiveDate>,
    pub plan_id: PlanId,
    pub plan_name: String,
    pub plan_description: Option<String>,
    pub plan_version_id: PlanVersionId,
    pub version: u32,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    // pub created_by_name: String,
    pub net_terms: u32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub activation_condition: SubscriptionActivationCondition,
    pub mrr_cents: u64,
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
    pub conn_meta: Option<ConnectionMeta>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub cycle_index: Option<u32>,
    pub status: SubscriptionStatusEnum,
    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    // Error tracking fields
    pub error_count: i32,
    pub last_error: Option<String>,
    pub next_retry: Option<NaiveDateTime>,
    // Quote to subscription linking
    pub quote_id: Option<QuoteId>,
    pub payment_methods_config: Option<PaymentMethodsConfig>,
}

pub enum CyclePosition {
    Start,
    End,
}

impl TryFrom<SubscriptionForDisplayRow> for Subscription {
    type Error = StoreErrorReport;

    fn try_from(val: SubscriptionForDisplayRow) -> Result<Self, Self::Error> {
        Ok(Subscription {
            id: val.subscription.id,
            customer_id: val.subscription.customer_id,
            customer_name: val.customer_name,
            customer_alias: val.customer_alias,
            invoicing_entity_id: val.invoicing_entity_id,
            billing_day_anchor: val.subscription.billing_day_anchor as u16,
            tenant_id: val.subscription.tenant_id,
            currency: val.subscription.currency,
            trial_duration: val.subscription.trial_duration.map(|x| x as u32),
            billing_start_date: val.subscription.billing_start_date,
            end_date: val.subscription.end_date,
            start_date: val.subscription.start_date,
            plan_id: val.plan_id,
            plan_name: val.plan_name,
            plan_description: val.plan_description,
            plan_version_id: val.subscription.plan_version_id,
            version: val.version as u32,
            created_at: val.subscription.created_at,
            created_by: val.subscription.created_by,
            net_terms: val.subscription.net_terms as u32,
            invoice_memo: val.subscription.invoice_memo,
            invoice_threshold: val.subscription.invoice_threshold,
            activated_at: val.subscription.activated_at,
            mrr_cents: val.subscription.mrr_cents as u64,
            period: val.subscription.period.into(),
            pending_checkout: val.subscription.pending_checkout,
            activation_condition: val.subscription.activation_condition.into(),
            current_period_start: val.subscription.current_period_start,
            current_period_end: val.subscription.current_period_end,
            conn_meta: val
                .subscription
                .conn_meta
                .map(TryInto::try_into)
                .transpose()?,
            cycle_index: val.subscription.cycle_index.map(|x| x as u32),
            status: val.subscription.status.into(),
            charge_automatically: val.subscription.charge_automatically,
            auto_advance_invoices: val.subscription.auto_advance_invoices,
            purchase_order: val.subscription.purchase_order,
            error_count: val.subscription.error_count,
            last_error: val.subscription.last_error,
            next_retry: val.subscription.next_retry,
            quote_id: val.subscription.quote_id,
            payment_methods_config: val
                .subscription
                .payment_methods_config
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| {
                    crate::errors::StoreError::SerdeError(
                        format!("Failed to parse payment_methods_config: {}", e),
                        e,
                    )
                })?,
        })
    }
}

// commitments etc will be represented by the Phases/Schedule, with possibly a way to simplify that in the UI (like trials that should also end up as a sort of phase, though it's a bit different as there's some conditional logic)
// activated = paid or considered as paid. If not activated, then the trial fallback applies
#[derive(Debug, Clone)]
pub struct SubscriptionNew {
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub created_by: Uuid,

    pub net_terms: Option<u32>, // 0 = due on issue, null = default to plan.net_terms TODO overrides
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,

    // when the subscription associated benefits should run from, trial included. Can be in the past (will be billed accordingly, for any past period)
    pub start_date: NaiveDate, // contract_start_date
    pub end_date: Option<NaiveDate>,

    // when the subscription should be billed from. Defaults to start_date + possible free trial period
    pub billing_start_date: Option<NaiveDate>,

    pub activation_condition: SubscriptionActivationCondition,

    // add trial config override ? ex: trial_end_date
    pub trial_duration: Option<u32>, // in days

    // if None, defaults to billing_start_date.day
    pub billing_day_anchor: Option<u16>,

    pub payment_methods_config: Option<PaymentMethodsConfig>,

    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
    pub backdate_invoices: bool,
    /// When true, prevents checkout session creation even with OnCheckout activation.
    /// Used when creating subscriptions from checkout completion (SelfServe flow).
    pub skip_checkout_session: bool,
}

pub struct SubscriptionNewEnriched<'a> {
    pub subscription: &'a SubscriptionNew,
    pub subscription_id: SubscriptionId,
    pub tenant_id: TenantId,
    pub period: BillingPeriodEnum,
    pub plan: &'a PlanForSubscription,
    pub payment_setup_result: &'a PaymentSetupResult,
    pub billing_day_anchor: u16,
    pub billing_start_date: NaiveDate,
    pub status: SubscriptionStatusEnum,
    pub current_period_start: NaiveDate,
    pub current_period_end: Option<NaiveDate>,
    pub next_cycle_action: Option<CycleActionEnum>,
    pub activated_at: Option<NaiveDateTime>,
    pub net_terms: u32,
    pub cycle_index: Option<u32>,
    pub quote_id: Option<QuoteId>,
    /// Effective trial duration: uses request override if provided, otherwise plan's trial_duration_days
    pub effective_trial_duration: Option<u32>,
}

impl SubscriptionNewEnriched<'_> {
    pub fn map_to_row(&self) -> Result<SubscriptionRowNew, StoreError> {
        let sub = &self.subscription;

        // pending_checkout controls billing logic (is_completed check) - always true for OnCheckout
        // skip_checkout_session is handled separately in persist_subscriptions_internal
        let pending_checkout = match sub.activation_condition {
            SubscriptionActivationCondition::OnCheckout => self.payment_setup_result.checkout,
            _ => false,
        };

        let payment_methods_config = sub
            .payment_methods_config
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError("Failed to serialize payment_methods_config".to_string(), e)
            })?;

        Ok(SubscriptionRowNew {
            id: self.subscription_id,
            trial_duration: self.effective_trial_duration.map(|x| x as i32),
            customer_id: sub.customer_id,
            billing_day_anchor: self.billing_day_anchor as i16,
            tenant_id: self.tenant_id,
            currency: self.plan.currency.clone(),
            billing_start_date: Some(self.billing_start_date),
            end_date: sub.end_date,
            plan_version_id: sub.plan_version_id,
            created_at: chrono::Utc::now().naive_utc(),
            created_by: sub.created_by,
            net_terms: self.net_terms as i32,
            invoice_memo: sub.invoice_memo.clone(),
            invoice_threshold: sub.invoice_threshold,
            activated_at: self.activated_at,
            mrr_cents: 0,
            period: self.period.into(),
            start_date: sub.start_date,
            activation_condition: sub.activation_condition.clone().into(),
            pending_checkout,
            status: self.status.clone().into(),
            current_period_start: self.current_period_start,
            current_period_end: self.current_period_end,
            next_cycle_action: self.next_cycle_action.clone(),
            cycle_index: self.cycle_index.map(|i| i as i32),
            charge_automatically: sub.charge_automatically,
            auto_advance_invoices: sub.auto_advance_invoices,
            purchase_order: sub.purchase_order.clone(),
            quote_id: self.quote_id,
            backdate_invoices: sub.backdate_invoices,
            payment_methods_config,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CreateSubscription {
    pub subscription: SubscriptionNew,
    pub price_components: Option<CreateSubscriptionComponents>,
    pub add_ons: Option<CreateSubscriptionAddOns>,
    pub coupons: Option<CreateSubscriptionCoupons>,
}

/// Components and add-ons are already processed (fees computed), so we skip plan-based processing.
#[derive(Debug, Clone)]
pub struct CreateSubscriptionFromQuote {
    pub subscription: SubscriptionNew,
    pub components: Vec<SubscriptionComponentNewInternal>,
    pub add_ons: Vec<SubscriptionAddOnNewInternal>,
    pub coupon_ids: Vec<CouponId>,
    pub quote_id: QuoteId,
}

/// Trial configuration from the plan version
#[derive(Debug, Clone)]
pub struct TrialConfig {
    pub duration_days: u32,
    pub is_free: bool,
    pub trialing_plan_id: Option<PlanId>,
    pub trialing_plan_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionDetails {
    pub subscription: Subscription,
    pub invoicing_entity: InvoicingEntity,
    pub customer: Customer,
    pub schedules: Vec<Schedule>,
    pub price_components: Vec<SubscriptionComponent>,
    pub add_ons: Vec<SubscriptionAddOn>,
    pub applied_coupons: Vec<AppliedCouponDetailed>,
    pub metrics: Vec<BillableMetric>,
    pub checkout_url: Option<String>,
    pub trial_config: Option<TrialConfig>,
}

#[derive(Clone, Debug)]
pub struct SubscriptionPatch {
    pub id: SubscriptionId,
    pub charge_automatically: Option<bool>,
    pub auto_advance_invoices: Option<bool>,
    pub net_terms: Option<u32>,
    pub invoice_memo: Option<Option<String>>,
    pub purchase_order: Option<Option<String>>,
    /// None = no change, Some(None) = reset to inherit, Some(Some(config)) = set config
    pub payment_methods_config: Option<Option<PaymentMethodsConfig>>,
}

golden::golden!(PaymentMethodsConfig, {
    "online_inherit" => PaymentMethodsConfig::online(),
    "online_card_only" => PaymentMethodsConfig::online_specific(true, false),
    "bank_transfer" => PaymentMethodsConfig::bank_transfer(),
    "external" => PaymentMethodsConfig::external()
});
