use chrono::{NaiveDate, NaiveDateTime};

use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionActivationCondition};
use crate::domain::subscription_add_ons::{CreateSubscriptionAddOns, SubscriptionAddOn};
use crate::domain::{
    AppliedCouponDetailed, BillableMetric, CreateSubscriptionComponents, CreateSubscriptionCoupons,
    Customer, InvoicingEntity, PlanForSubscription, Schedule, SubscriptionComponent,
    SubscriptionStatusEnum,
};
use crate::errors::StoreErrorReport;
use crate::services::PaymentSetupResult;
use common_domain::ids::{
    BankAccountId, CustomerConnectionId, CustomerId, InvoicingEntityId, PlanId, PlanVersionId,
    SubscriptionId, TenantId,
};
use diesel_models::enums::CycleActionEnum;
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
    pub card_connection_id: Option<CustomerConnectionId>,
    pub direct_debit_connection_id: Option<CustomerConnectionId>,
    pub bank_account_id: Option<BankAccountId>,
    pub plan_id: PlanId,
    pub plan_name: String,
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
        })
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

    // API only. describes how the subscription should be billed.
    // Auto is default : uses the existing default method for customer, or attempts a checkout if invoicing entity's PP, or link to bank, or set as external payment
    // ==> try to simplify TODO
    pub payment_strategy: Option<SubscriptionPaymentStrategy>,

    pub auto_advance_invoices: bool,
    pub charge_automatically: bool,
    pub purchase_order: Option<String>,
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
}

impl SubscriptionNewEnriched<'_> {
    pub fn map_to_row(&self) -> SubscriptionRowNew {
        let sub = &self.subscription;
        SubscriptionRowNew {
            id: self.subscription_id,
            trial_duration: sub.trial_duration.map(|x| x as i32),
            customer_id: sub.customer_id,
            billing_day_anchor: self.billing_day_anchor as i16,
            tenant_id: self.tenant_id,
            currency: self.plan.currency.clone(),
            billing_start_date: Some(self.billing_start_date),
            end_date: sub.end_date,
            plan_version_id: sub.plan_version_id,
            created_at: chrono::Utc::now().naive_utc(),
            card_connection_id: self.payment_setup_result.card_connection_id,
            direct_debit_connection_id: self.payment_setup_result.direct_debit_connection_id,
            bank_account_id: self.payment_setup_result.bank,
            created_by: sub.created_by,
            net_terms: self.net_terms as i32,
            invoice_memo: sub.invoice_memo.clone(),
            invoice_threshold: sub.invoice_threshold,
            activated_at: self.activated_at,
            mrr_cents: 0,
            period: self.period.clone().into(),
            start_date: sub.start_date,
            activation_condition: sub.activation_condition.clone().into(),
            payment_method: self.payment_setup_result.payment_method,
            pending_checkout: self.payment_setup_result.checkout,
            status: self.status.clone().into(),
            current_period_start: self.current_period_start,
            current_period_end: self.current_period_end,
            next_cycle_action: self.next_cycle_action.clone(),
            cycle_index: self.cycle_index.map(|i| i as i32),
            charge_automatically: sub.charge_automatically,
            auto_advance_invoices: sub.auto_advance_invoices,
            purchase_order: sub.purchase_order.clone(),
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
    pub invoicing_entity: InvoicingEntity,
    pub customer: Customer,
    pub schedules: Vec<Schedule>,
    pub price_components: Vec<SubscriptionComponent>,
    pub add_ons: Vec<SubscriptionAddOn>,
    pub applied_coupons: Vec<AppliedCouponDetailed>,
    pub metrics: Vec<BillableMetric>,
    pub checkout_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionInvoiceCandidate {
    pub id: SubscriptionId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub plan_version_id: PlanVersionId,
    pub plan_name: String,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub billing_start_date: Option<NaiveDate>,
    pub billing_day_anchor: i16,
    pub net_terms: i32,
    pub activated_at: Option<NaiveDateTime>,
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
            // plan_id: self.plan_version.plan_id,
            plan_name: val.plan_version.plan_name,
            currency: val.plan_version.currency,
            net_terms: val.subscription.net_terms,
            // version: self.plan_version.version,
            period: val.subscription.period.into(),
        }
    }
}
