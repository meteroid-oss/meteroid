use chrono::{Datelike, NaiveDate, NaiveDateTime};

use crate::domain::enums::{BillingPeriodEnum, SubscriptionActivationCondition};
use crate::domain::subscription_add_ons::{CreateSubscriptionAddOns, SubscriptionAddOn};
use crate::domain::{
    AppliedCouponDetailed, BillableMetric, CreateSubscriptionComponents, CreateSubscriptionCoupons,
    PlanForSubscription, Schedule, SubscriptionComponent,
};
use crate::repositories::subscriptions::PaymentSetupResult;
use common_domain::ids::{
    BankAccountId, BaseId, CustomerConnectionId, CustomerId, PlanId, SubscriptionId, TenantId,
};
use diesel_models::subscriptions::SubscriptionRowNew;
use diesel_models::subscriptions::{
    SubscriptionForDisplayRow, SubscriptionInvoiceCandidateRow, SubscriptionRow,
};
use o2o::o2o;
use uuid::Uuid;

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
    pub plan_version_id: Uuid,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub net_terms: i32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
    pub mrr_cents: i64,
    #[from(~.into())]
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
    #[ghost({None})]
    pub checkout_token: Option<String>,
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
    pub card_connection_id: Option<CustomerConnectionId>,
    pub direct_debit_connection_id: Option<CustomerConnectionId>,
    pub bank_account_id: Option<BankAccountId>,
    pub plan_id: PlanId,
    pub plan_name: String,
    pub plan_version_id: Uuid,
    pub version: u32,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    // pub created_by_name: String,
    pub net_terms: u32,
    pub invoice_memo: Option<String>,
    pub invoice_threshold: Option<rust_decimal::Decimal>,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub cancellation_reason: Option<String>,
    pub mrr_cents: u64,
    pub period: BillingPeriodEnum,
    pub pending_checkout: bool,
}

impl From<SubscriptionForDisplayRow> for Subscription {
    fn from(val: SubscriptionForDisplayRow) -> Self {
        Subscription {
            id: val.subscription.id,
            customer_id: val.subscription.customer_id,
            customer_name: val.customer_name,
            customer_alias: val.customer_alias,
            billing_day_anchor: val.subscription.billing_day_anchor as u16,
            tenant_id: val.subscription.tenant_id,
            currency: val.subscription.currency,
            trial_duration: val.subscription.trial_duration.map(|x| x as u32),
            billing_start_date: val.subscription.billing_start_date,
            end_date: val.subscription.end_date,
            start_date: val.subscription.start_date,
            plan_id: val.plan_id,
            plan_name: val.plan_name,
            plan_version_id: val.subscription.plan_version_id,
            card_connection_id: val.subscription.card_connection_id,
            direct_debit_connection_id: val.subscription.direct_debit_connection_id,
            bank_account_id: val.subscription.bank_account_id,
            version: val.version as u32,
            created_at: val.subscription.created_at,
            created_by: val.subscription.created_by,
            net_terms: val.subscription.net_terms as u32,
            invoice_memo: val.subscription.invoice_memo,
            invoice_threshold: val.subscription.invoice_threshold,
            activated_at: val.subscription.activated_at,
            canceled_at: val.subscription.canceled_at,
            cancellation_reason: val.subscription.cancellation_reason,
            mrr_cents: val.subscription.mrr_cents as u64,
            period: val.subscription.period.into(),
            pending_checkout: val.subscription.pending_checkout,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SubscriptionPaymentStrategy {
    Auto, // uses the existing method if exist, do card checkout if standard plan & configured provider, else bank if exists else external
    // Checkout, // forces a checkout, even if the user already has a card on file. Checkout can basically be a validation step.
    Bank, // TODO not a strategy ? we just add the bank_id to the subscription & invoice
    External,
    // TODO
    // CustomerPaymentMethod(id)
    // PaymentProvider(id)
    // Bank(id)
}

// commitments etc will be represented by the Phases/Schedule, with possibly a way to simplify that in the UI (like trials that should also end up as a sort of phase, though it's a bit different as there's some conditional logic)
// activated = paid or considered as paid. If not activated, then the trial fallback applies
#[derive(Debug, Clone)]
pub struct SubscriptionNew {
    pub customer_id: CustomerId,
    pub plan_version_id: Uuid,
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

    // API only. describes how the subscription should be billed.
    // Auto is default : uses the existing default method for customer, or attempts a checkout if invoicing entity's PP, or link to bank, or set as external payment
    // ==> try to simplify TODO
    pub payment_strategy: Option<SubscriptionPaymentStrategy>,
}

impl SubscriptionNew {
    pub fn map_to_row(
        &self,
        period: BillingPeriodEnum,
        tenant_id: TenantId,
        plan: &PlanForSubscription,
        payment_setup_result: &PaymentSetupResult,
    ) -> SubscriptionRowNew {
        // in the current state we set billing_date/day even if free.
        // That is because a free plan could still have included usage
        // TODO : => decide to make it mandatory or not in db
        let billing_start_date = self.billing_start_date.unwrap_or(self.start_date);
        let billing_day_anchor = self
            .billing_day_anchor
            .unwrap_or_else(|| self.billing_start_date.unwrap_or(self.start_date).day() as u16);
        let net_terms = self.net_terms.unwrap_or(plan.net_terms as u32);

        let activated_at = match self.activation_condition {
            SubscriptionActivationCondition::OnStart => self.start_date.and_hms_opt(0, 0, 0),
            _ => None,
        };

        let now = chrono::Utc::now().naive_utc();

        // TODO subscription trial does not inherit from plan trial

        SubscriptionRowNew {
            id: SubscriptionId::new(),
            trial_duration: self.trial_duration.map(|x| x as i32),
            customer_id: self.customer_id,
            billing_day_anchor: billing_day_anchor as i16,
            tenant_id,
            currency: plan.currency.clone(),
            billing_start_date: Some(billing_start_date),
            end_date: self.end_date,
            plan_version_id: self.plan_version_id,
            created_at: now,
            card_connection_id: payment_setup_result.card_connection_id,
            direct_debit_connection_id: payment_setup_result.direct_debit_connection_id,
            bank_account_id: payment_setup_result.bank,
            created_by: self.created_by,
            net_terms: net_terms as i32,
            invoice_memo: self.invoice_memo.clone(),
            invoice_threshold: self.invoice_threshold,
            activated_at,
            mrr_cents: 0,
            period: period.into(),
            start_date: self.start_date,
            activation_condition: self.activation_condition.clone().into(),
            payment_method: payment_setup_result.payment_method,
            pending_checkout: payment_setup_result.checkout,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateSubscription {
    pub subscription: SubscriptionNew,
    pub price_components: Option<CreateSubscriptionComponents>,
    pub add_ons: Option<CreateSubscriptionAddOns>,
    pub coupons: Option<CreateSubscriptionCoupons>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionDetails {
    pub subscription: Subscription,
    pub schedules: Vec<Schedule>,
    pub price_components: Vec<SubscriptionComponent>,
    pub add_ons: Vec<SubscriptionAddOn>,
    pub applied_coupons: Vec<AppliedCouponDetailed>,
    pub metrics: Vec<BillableMetric>,
    pub checkout_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionInvoiceCandidate {
    pub id: SubscriptionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: Uuid,
    pub plan_name: String,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: i16,
    pub net_terms: i32,
    pub activated_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub currency: String,
    pub period: BillingPeriodEnum,
}

impl From<SubscriptionInvoiceCandidateRow> for SubscriptionInvoiceCandidate {
    fn from(val: SubscriptionInvoiceCandidateRow) -> Self {
        SubscriptionInvoiceCandidate {
            id: val.subscription.id,
            tenant_id: val.subscription.tenant_id,
            customer_id: val.subscription.customer_id,
            plan_version_id: val.subscription.plan_version_id,
            start_date: val.subscription.start_date,
            billing_start_date: val.subscription.billing_start_date,
            end_date: val.subscription.end_date,
            billing_day_anchor: val.subscription.billing_day_anchor,
            activated_at: val.subscription.activated_at,
            canceled_at: val.subscription.canceled_at,
            // plan_id: self.plan_version.plan_id,
            plan_name: val.plan_version.plan_name,
            currency: val.plan_version.currency,
            net_terms: val.subscription.net_terms,
            // version: self.plan_version.version,
            period: val.subscription.period.into(),
        }
    }
}
