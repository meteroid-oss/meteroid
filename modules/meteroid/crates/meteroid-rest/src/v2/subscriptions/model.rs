//! The subscription resources of API v2 (ADR-0013 §10): what a subscription reads as, what
//! creating one and changing one take, and what a change previews and answers.
use crate::v2::charges::{Cadence, Charge, PriceModel};
use crate::v2::documents::{DocumentCoupon, DocumentLine, DocumentTax};
use crate::v2::entitlements::model::{Entitlement, EntitlementInput};
use crate::v2::plans::Minimum;
pub use crate::v2::refs::{AddOnRef, CouponRef, CreditNoteRef, CustomerRef, InvoiceRef, PlanRef};
use chrono::NaiveDate;
use common_domain::ids::{ScheduledEventId, SubscriptionId};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::SubscriptionStatusEnum;
use meteroid_store::domain::enums::SubscriptionActivationCondition;
use meteroid_store::domain::subscription_changes::ChangeDirection;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

// ── Reading ─────────────────────────────────────────────────────────

/// Where the subscription is in its life: `pending_activation` before its start, or waiting for
/// its checkout or a manual activation; `trial_active` in its trial; `active` billing;
/// `trial_expired` a free trial ended without the checkout that starts billing; `cancelled`
/// ended by a cancellation; `aborted` cancelled before it was ever active; `completed` ended on
/// its end date.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    /// Before its start, or waiting for its checkout or a manual activation.
    PendingActivation,
    TrialActive,
    Active,
    /// A free trial ended without the checkout that starts billing.
    TrialExpired,
    Cancelled,
    /// Cancelled before it was ever active.
    Aborted,
    /// Ended on its end date.
    Completed,
}

impl From<SubscriptionStatusEnum> for SubscriptionStatus {
    fn from(status: SubscriptionStatusEnum) -> Self {
        match status {
            SubscriptionStatusEnum::PendingActivation => SubscriptionStatus::PendingActivation,
            SubscriptionStatusEnum::TrialActive => SubscriptionStatus::TrialActive,
            SubscriptionStatusEnum::Active => SubscriptionStatus::Active,
            SubscriptionStatusEnum::TrialExpired => SubscriptionStatus::TrialExpired,
            SubscriptionStatusEnum::Cancelled => SubscriptionStatus::Cancelled,
            SubscriptionStatusEnum::Aborted => SubscriptionStatus::Aborted,
            SubscriptionStatusEnum::Completed => SubscriptionStatus::Completed,
        }
    }
}

impl From<SubscriptionStatus> for SubscriptionStatusEnum {
    fn from(status: SubscriptionStatus) -> Self {
        match status {
            SubscriptionStatus::PendingActivation => SubscriptionStatusEnum::PendingActivation,
            SubscriptionStatus::TrialActive => SubscriptionStatusEnum::TrialActive,
            SubscriptionStatus::Active => SubscriptionStatusEnum::Active,
            SubscriptionStatus::TrialExpired => SubscriptionStatusEnum::TrialExpired,
            SubscriptionStatus::Cancelled => SubscriptionStatusEnum::Cancelled,
            SubscriptionStatus::Aborted => SubscriptionStatusEnum::Aborted,
            SubscriptionStatus::Completed => SubscriptionStatusEnum::Completed,
        }
    }
}

/// What starts the subscription: `on_start` its start date; `on_checkout` the customer's
/// checkout; `manual` an operator, from the dashboard.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    /// Its start date.
    #[default]
    OnStart,
    /// The customer's checkout.
    OnCheckout,
    /// An operator, from the dashboard.
    Manual,
}

impl From<Activation> for SubscriptionActivationCondition {
    fn from(activation: Activation) -> Self {
        match activation {
            Activation::OnStart => SubscriptionActivationCondition::OnStart,
            Activation::OnCheckout => SubscriptionActivationCondition::OnCheckout,
            Activation::Manual => SubscriptionActivationCondition::Manual,
        }
    }
}

impl From<SubscriptionActivationCondition> for Activation {
    fn from(condition: SubscriptionActivationCondition) -> Self {
        match condition {
            SubscriptionActivationCondition::OnStart => Activation::OnStart,
            SubscriptionActivationCondition::OnCheckout => Activation::OnCheckout,
            SubscriptionActivationCondition::Manual => Activation::Manual,
        }
    }
}

/// The subscription's dates. Days are ISO 8601 dates, moments RFC 3339.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Dates {
    /// When its benefits start, a trial included.
    pub start: NaiveDate,
    /// When it ends; none: until cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<NaiveDate>,
    /// When it is billed from: its start, after a free trial.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_start: Option<NaiveDate>,
    /// When it was first activated (its start, its checkout or a manual activation); absent
    /// before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<ApiDateTime>,
    /// When it ended (cancelled, completed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<ApiDateTime>,
}

/// The billing period the subscription is in.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CurrentPeriod {
    /// Its first day.
    pub start: NaiveDate,
    /// Exclusive; none for a subscription not billed by period yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<NaiveDate>,
}

/// The subscription's trial, the free period before it is billed, as recorded at its start;
/// absent when it has none. Lists carry its `days`; the rest is read on a single subscription.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TrialState {
    /// Its length in days, from its start.
    pub days: u32,
    /// Its first day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_on: Option<NaiveDate>,
    /// Exclusive: the day after its last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_on: Option<NaiveDate>,
    /// The day billing starts when the subscription converts: its end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_starts_on: Option<NaiveDate>,
    /// The plan whose features it gives over the subscription's own; none: its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features_of: Option<PlanRef>,
    /// The trial offer it came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<TrialOfferSummary>,
    /// Where it stands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::v2::trials::TrialStatus>,
    /// How it ended; absent while it runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<crate::v2::trials::TrialOutcome>,
    /// The day it ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_on: Option<NaiveDate>,
    /// What its end does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_end: Option<crate::v2::trials::TrialAtEnd>,
    /// What a converting end does if the subscription is charged automatically and has no
    /// payment method then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub without_card: Option<crate::v2::trials::TrialWithoutCard>,
    /// Whether the subscription has a payment method it is charged or invoiced with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_payment_method: Option<bool>,
    /// What its end is estimated to charge first, while it runs and converts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_charge_estimate: Option<crate::v2::trials::FirstChargeEstimate>,
}

/// A trial offer, as a subscription or a plan names it.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TrialOfferSummary {
    /// The offer's id.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    #[schema(value_type = String)]
    pub id: common_domain::ids::TrialOfferId,
    /// Its code, when it has one.
    pub code: Option<String>,
    /// Its name.
    pub name: String,
}

/// A trial offer, by its id or its code: its days, and what follows it. Refused when the
/// customer already had it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OfferTrial {
    /// The offer, by id or code.
    pub offer: String,
}

/// A trial of its own terms: `days` or an end date `until` (exclusive), one of them. It bills
/// nothing and converts at its end.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomTrial {
    /// 1 to 3,650.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    /// After the billing start, 3,650 days at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<NaiveDate>,
    /// The plan whose features it gives, by code or id; the plan's trial's when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features_of: Option<String>,
}

/// No trial.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct NoTrial {}

/// The plan's trial, asked: refused when the customer already had it. Left out, the plan's
/// trial is given unless the customer already had it.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct TrialOfThePlan {}

/// The trial a new subscription starts with: the plan's, none, an offer, or one of its own
/// terms. A trial is free.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schema(as = SubscriptionTrialInput)]
pub enum TrialInput {
    Plan(TrialOfThePlan),
    None(NoTrial),
    Offer(OfferTrial),
    Custom(CustomTrial),
}

/// A trial's new end, later than its current one: a day, or days added to its end. One of the two.
#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExtendTrial {
    /// The new end, exclusive.
    #[serde(default)]
    pub until: Option<NaiveDate>,
    /// Days added to the current end, at least one.
    #[serde(default)]
    pub days: Option<u32>,
}

/// What ending a trial today would do, nothing written.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct TrialEndPreview {
    /// The day it would end: today.
    pub effective_date: NaiveDate,
    /// How it would end: its end action applied today (a manual one converts).
    pub outcome: crate::v2::trials::TrialOutcome,
    /// What it would issue today: the opening invoice of a conversion, or what a switch of plan
    /// issues.
    pub documents: Vec<Document>,
    /// True when a part of the preview could not be priced and is left out (`missing` names it).
    pub incomplete: bool,
    /// What is left out, when `incomplete`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<PreviewPart>,
    /// The subscription as it would read once its trial ended.
    pub subscription_after: Subscription,
}

/// A count to come: a decrease waiting for the period's end, an increase waiting for its payment
/// or for the next period.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct PendingQuantity {
    /// The count once it applies.
    #[schema(value_type = String, format = "decimal", example = "10")]
    pub value: Decimal,
    /// When it applies; possibly past for an increase waiting for its payment.
    pub effective_at: ApiDateTime,
}

/// A managed quantity the subscription holds (seats).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct QuantityState {
    /// The count in force now, as a decimal string.
    #[schema(value_type = String, format = "decimal", example = "12")]
    pub held: Decimal,
    /// The counts to come, soonest first, each the count once it applies.
    pub pending: Vec<PendingQuantity>,
}

/// Where a charge of the subscription comes from: `plan` a charge of its plan, its price the
/// plan's or overridden for this subscription; `add_on` one of its add-ons; `amendment` added to
/// this subscription beside its plan, at its creation or by an amendment.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChargeOrigin {
    /// A charge of its plan, its price the plan's or overridden for this subscription.
    Plan,
    /// One of its add-ons.
    AddOn,
    /// Added to this subscription beside its plan, at its creation or by an amendment.
    Amendment,
}

/// A charge the subscription bills, as a plan's charge reads: its one price is the one this
/// subscription is billed (`*`, in its currency and cadence).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubscriptionCharge {
    /// Where the charge comes from.
    pub origin: ChargeOrigin,
    /// The add-on of a charge from an add-on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_on: Option<AddOnRef>,
    #[serde(flatten)]
    pub charge: Charge,
}

/// An add-on the subscription holds.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubscribedAddOn {
    /// The add-on held.
    pub add_on: AddOnRef,
    /// Its name on invoices.
    pub name: String,
    /// Instances held.
    pub quantity: i32,
}

/// A coupon redeemed on the subscription.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct AppliedCoupon {
    /// The coupon redeemed.
    pub coupon: CouponRef,
    /// Still discounting.
    pub active: bool,
    /// Invoices it discounted; absent before the first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_count: Option<i32>,
}

/// What a scheduled change does: `plan_change` moves the subscription to another plan, option
/// or version; `cancellation` ends it; `amendment` adds or removes charges and add-ons.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledChangeType {
    PlanChange,
    Cancellation,
    Amendment,
    /// A phase's start (Enterprise): its terms apply from that day.
    Phase,
}

/// The names of what a scheduled amendment adds and removes.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Default)]
pub struct AmendmentSummary {
    /// The names of the charges it adds.
    pub added_charges: Vec<String>,
    /// The names of the charges it removes.
    pub removed_charges: Vec<String>,
    /// The names of the add-ons it adds.
    pub added_add_ons: Vec<String>,
    /// The names of the add-ons it removes.
    pub removed_add_ons: Vec<String>,
}

/// A change waiting for its date.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct ScheduledChange {
    /// The scheduled change's id, as a change made answers it in `scheduled_change`.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ScheduledEventId,
    /// What it does.
    #[serde(rename = "type")]
    pub kind: ScheduledChangeType,
    /// The day it takes effect.
    pub effective_date: NaiveDate,
    /// The plan a plan change moves to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanRef>,
    /// A cancellation's reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Asked by the customer, from the portal, rather than by the tenant.
    pub initiated_by_customer: bool,
    /// What an amendment adds and removes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amendment: Option<AmendmentSummary>,
}

/// The changes of a subscription waiting for their date.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Schedule {
    /// Soonest first.
    pub changes: Vec<ScheduledChange>,
}

/// A cancellation waiting for its date: revoke it with `POST …/cancel/revoke`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Cancellation {
    /// The day the subscription ends.
    pub effective_date: NaiveDate,
    /// The reason sent with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Asked by the customer, from the portal, rather than by the tenant.
    pub initiated_by_customer: bool,
}

/// An amount in minor units of its currency.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct Money {
    /// In minor units: `4900` is 49.00.
    pub amount: i64,
    /// ISO 4217.
    pub currency: String,
}

/// How the subscription is invoiced.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Invoicing {
    /// Days an invoice is due after it is issued.
    pub net_terms: u32,
    /// The customer's purchase order number, printed on every invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purchase_order: Option<String>,
    /// Printed on every invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// Invoices are finalized and sent without a review.
    pub auto_finalize: bool,
    /// Invoices are charged to the customer's payment method.
    pub charge_automatically: bool,
}

pub use crate::v2::customers::resources::{ResourceName, ResourceRef};

/// A subscription. Lists carry what the subscription itself holds; `quantities`, `levels`,
/// `charges`, `add_ons`, `coupons`, `schedule`, `cancellation` and `minimum` are read on a single
/// one.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Subscription {
    /// The subscription's id, `sub_…`.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: SubscriptionId,
    /// The integrator's own id, unique per tenant; paths take it in place of the id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// The customer billed.
    pub customer: CustomerRef,
    /// The plan, with the version and option it is on.
    pub plan: PlanRef,
    /// Where it is in its life.
    pub status: SubscriptionStatus,
    /// What it is billed in (ISO 4217).
    pub currency: String,
    /// How often it renews.
    pub period: Cadence,
    /// What starts it.
    pub activation: Activation,
    /// The day of the month its periods start on, 1 to 31 (a shorter month starts its period on
    /// its last day).
    pub billing_day_anchor: u16,
    /// Its start, end, billing start, activation and ending.
    pub dates: Dates,
    /// The billing period it is in.
    pub current_period: CurrentPeriod,
    /// Its trial; absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<TrialState>,
    /// The managed quantities it holds (seats), by code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantities: Option<BTreeMap<String, QuantityState>>,
    /// The level of each usage-levels charge, by product code, as the usage it includes:
    /// `{"api": "10000"}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<BTreeMap<String, String>>,
    /// The charges it bills: its plan's, its add-ons' and those added to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charges: Option<Vec<SubscriptionCharge>>,
    /// The add-ons it holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_ons: Option<Vec<SubscribedAddOn>>,
    /// The coupons redeemed on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupons: Option<Vec<AppliedCoupon>>,
    /// What the offers applied to it give: each grant with its window, where it came from and
    /// what it wrote; absent when it has none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<crate::v2::offers::SubscriptionGrant>,
    /// The changes waiting for their date, soonest first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<ScheduledChange>>,
    /// The cancellation waiting for its date; absent when none is scheduled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<Cancellation>,
    /// The floor on what it is billed each period; absent when it has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Minimum>,
    /// Monthly recurring revenue, tax excluded.
    pub mrr: Money,
    /// Its prices are quoted tax included (its plan version's, when it was created).
    pub tax_inclusive: bool,
    /// The stored resource it is for (a workspace, a project), fixed once active: it bills the
    /// metrics attributed to that resource's type, for the events carrying its key value. None:
    /// the whole customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceRef>,
    /// Its own entitlement rows, the subscription tier (`set` only); read on a single one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entitlement_overrides: Option<Vec<Entitlement>>,
    /// Read whole, what it is sold with as it stands: `plan_sold_for_mismatch` (the plan is sold
    /// for another resource type than the subscription's resource, or for the customer),
    /// `metric_not_attributed` (its resource never bills some metrics it prices).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// With `metric_not_attributed`: the codes of the metrics it prices that are not attributed
    /// to its resource's type, so it bills none of their usage.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub metrics_not_attributed: Vec<String>,
    /// How it is invoiced.
    pub invoicing: Invoicing,
    /// Values of the tenant's subscription properties, by key.
    #[schema(value_type = HashMap<String, Value>)]
    pub custom_properties: serde_json::Value,
    /// When it was created.
    pub created_at: ApiDateTime,
    /// When it, or anything read of it, last changed.
    pub updated_at: ApiDateTime,
}

crate::v2_list!(SubscriptionList, Subscription);

/// The trials a list narrows to.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrialListParam {
    Ending,
    AwaitingDecision,
    AwaitingPayment,
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct SubscriptionListQuery {
    /// Only this customer's: its id, or `ext:<alias>`.
    pub customer: Option<String>,
    /// Only those on this plan, by code or id.
    pub plan: Option<String>,
    /// Only those in one of these statuses: repeat the parameter for several
    /// (`?status=active&status=trial_active`).
    #[param(style = Form, explode)]
    pub status: Option<Vec<SubscriptionStatus>>,
    /// Only those changed after this moment (RFC 3339).
    #[param(format = DateTime)]
    pub updated_after: Option<String>,
    /// Only those created after this moment (RFC 3339).
    #[param(format = DateTime)]
    pub created_after: Option<String>,
    /// Only those created before this moment (RFC 3339).
    #[param(format = DateTime)]
    pub created_before: Option<String>,
    /// Only those whose trial is ending (running, ending within `trial_ends_within_days`),
    /// awaiting a decision or awaiting payment.
    pub trial: Option<TrialListParam>,
    /// With `trial=ending`: within this many days from today, 1 to 3650; 7 by default.
    pub trial_ends_within_days: Option<u32>,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

// ── Creating ────────────────────────────────────────────────────────

/// How the subscription is invoiced.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct InvoicingInput {
    /// Days an invoice is due after it is issued, 0 to 365; the plan's when left out (due on
    /// receipt when it sells credit).
    #[serde(default)]
    pub net_terms: Option<u32>,
    /// The customer's purchase order number, printed on every invoice.
    #[serde(default)]
    pub purchase_order: Option<String>,
    /// Printed on every invoice.
    #[serde(default)]
    pub memo: Option<String>,
    /// Invoices are finalized and sent without a review; true when left out.
    #[serde(default)]
    pub auto_finalize: Option<bool>,
    /// Invoices are charged to the customer's payment method; true when left out: the invoicing
    /// entity then needs a payment provider.
    #[serde(default)]
    pub charge_automatically: Option<bool>,
}

/// Another price for one charge of the subscription: the charge keeps how it bills its product.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ChargeOverride {
    /// The charge, by its product's code (or id).
    pub charge: String,
    /// Its name on invoices; unchanged when left out.
    #[serde(default)]
    pub name: Option<String>,
    /// Its price for this subscription, in the subscription's currency and major units.
    pub price: PriceModel,
    /// The charge's cadence when left out.
    #[serde(default)]
    pub cadence: Option<Cadence>,
}

/// How a new subscription's charges differ from its plan's.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CreateCharges {
    /// Plan charges at another price, 100 at most.
    #[serde(default, rename = "override")]
    pub overrides: Vec<ChargeOverride>,
    /// Charges beside the plan's, each with one price in the subscription's currency, 100 at
    /// most.
    #[serde(default)]
    pub add: Vec<Charge>,
    /// Plan charges left out, by product code.
    #[serde(default)]
    pub remove: Vec<String>,
}

fn one() -> u32 {
    1
}

/// An add-on bought with the subscription, or added by an amendment.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AddOnPurchase {
    /// The add-on, by code or id.
    pub add_on: String,
    /// Instances held, 1 to 2147483647; 1 when left out.
    #[serde(default = "one")]
    pub quantity: u32,
}

/// A subscription to create: a customer on a published version of a plan, in one of its
/// options, with its seats, levels, price overrides, own charges, add-ons and coupons.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateSubscription {
    /// The customer's id, or its alias as `ext:<alias>`.
    pub customer: String,
    /// The plan, by code or id.
    pub plan: String,
    /// The option's alias; the first the currency sells when left out.
    #[serde(default)]
    pub option: Option<String>,
    /// A published version's number; the active one when left out.
    #[serde(default)]
    pub version: Option<i32>,
    /// Checked when sent: a subscription is sold in its customer's currency when the plan sells
    /// it, else in the plan's.
    #[serde(default)]
    pub currency: Option<String>,
    /// The integrator's own id, unique per tenant, 1 to 255 bytes; paths take it in place of the
    /// id. One taken answers `409 code_taken`.
    #[serde(default)]
    pub external_id: Option<String>,
    /// A stored resource of the customer to subscribe for, `{"id": …}` or
    /// `{"type": …, "key": …}`; the whole customer when left out.
    #[serde(default)]
    pub resource: Option<ResourceName>,
    /// The subscription's own entitlement rows, above its plan's (`set` only).
    #[serde(default)]
    pub entitlement_overrides: Vec<EntitlementInput>,
    /// The day it starts; today when left out. A past date bills the periods since.
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    /// The day it stops renewing and completes; it renews until cancelled when left out.
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    /// What it is given beside its plan: the plan's start offer when left out, unless the
    /// customer already had it.
    #[serde(default)]
    pub offer: Option<crate::v2::offers::OfferInput>,
    /// The plan's trial when left out, unless the customer already had it.
    #[serde(default)]
    pub trial: Option<TrialInput>,
    /// The day of the month periods start on, 1 to 31; the start date's when left out.
    #[serde(default)]
    pub billing_day_anchor: Option<u16>,
    /// What starts it; `on_start` when left out.
    #[serde(default)]
    pub activation: Activation,
    /// The starting count of each managed quantity its seat charges count, by code: `{"users": "12"}`.
    #[serde(default)]
    pub quantities: BTreeMap<String, String>,
    /// The level of each usage-levels charge, by product code, as the usage it includes:
    /// `{"api": "10000"}`. The lowest when left out.
    #[serde(default)]
    pub levels: BTreeMap<String, String>,
    /// How its charges differ from its plan's: prices overridden, charges added or left out.
    #[serde(default)]
    pub charges: CreateCharges,
    /// Add-ons bought with it, 100 at most.
    #[serde(default)]
    pub add_ons: Vec<AddOnPurchase>,
    /// Coupon codes, 20 at most.
    #[serde(default)]
    pub coupons: Vec<String>,
    /// How it is invoiced.
    #[serde(default)]
    pub invoicing: InvoicingInput,
    /// Validated against the tenant's custom property definitions.
    #[serde(default)]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

// ── Changing ────────────────────────────────────────────────────────

/// When a change takes effect: `immediate` now, prorated; `period_end` at the end of the current
/// period; `date` on `date` (cancellations only).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WhenType {
    /// Now, prorated.
    Immediate,
    /// At the end of the current period.
    PeriodEnd,
    /// On `date` (cancellations).
    Date,
}

/// When a change takes effect. Which a change accepts is checked in one place (`when.rs`): a plan
/// change, an amendment or a level move take effect now or at the period's end, a downgrade of
/// plan or level at the period's end only; a cancellation also on a date.
#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// When it takes effect.
    #[serde(rename = "type")]
    pub kind: WhenType,
    /// The day it takes effect, with `type: date` only (required then).
    #[serde(default)]
    pub date: Option<NaiveDate>,
}

/// A move to another plan, option or version.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = ChangePlan)]
#[serde(deny_unknown_fields)]
pub struct PlanChangeInput {
    /// The plan moved to, by code or id.
    pub plan: String,
    /// The option's alias; the one of the same alias when the plan has it, else the first sold.
    #[serde(default)]
    pub option: Option<String>,
    /// A published version's number; the active one when left out.
    #[serde(default)]
    pub version: Option<i32>,
    /// Seat counts on the new plan, by managed quantity code; the counts held when left out.
    #[serde(default)]
    pub quantities: BTreeMap<String, String>,
    /// Levels on the new plan, by product code; the level held when the new plan sells it.
    #[serde(default)]
    pub levels: BTreeMap<String, String>,
    /// Prices on the new plan, by product code: a charge priced by one amount (a rate, a seat,
    /// a capacity) at its own price, a decimal in the subscription's currency, over the list
    /// price.
    #[serde(default)]
    pub prices: BTreeMap<String, String>,
    /// Required: `immediate` or `period_end`; a downgrade at the period's end only.
    #[serde(default)]
    pub when: Option<When>,
}

/// When the seats an increase adds are in force: `immediate` at once, their prorated invoice
/// issued beside them as the subscription's invoicing says; `on_payment` once the invoice billing
/// them is paid, the change `pending_payment` until then (an increase billing nothing applies at
/// once).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SeatActivation {
    /// At once, their prorated invoice issued beside them as the subscription's invoicing says.
    #[default]
    Immediate,
    /// Once the invoice billing them is paid: until then the change is `pending_payment`. An
    /// increase billing nothing applies at once.
    OnPayment,
}

/// A new count of a managed quantity: `set` or `delta`, not both.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[schema(as = ChangeQuantity)]
#[serde(deny_unknown_fields)]
pub struct QuantityChangeInput {
    /// The count to hold.
    #[serde(default)]
    #[schema(example = "12")]
    pub set: Option<String>,
    /// The change of the count: `"+1"`, `"-2"`.
    #[serde(default)]
    #[schema(example = "+1")]
    pub delta: Option<String>,
    /// When an increase is in force: `immediate` (the default) or `on_payment` of its invoice. A
    /// decrease follows the quantity's policy either way.
    #[serde(default)]
    pub activation: SeatActivation,
}

/// A move of a usage-levels charge to another of its levels.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = ChangeLevel)]
#[serde(deny_unknown_fields)]
pub struct LevelChangeInput {
    /// The usage the level includes, one of the charge's levels.
    #[schema(format = "decimal", example = "10000")]
    pub included: String,
    /// A higher level applies `immediate`, a lower one at `period_end`; checked when sent, and
    /// as the direction says when left out.
    #[serde(default)]
    pub when: Option<When>,
}

/// How an amendment changes the subscription's charges.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct ChargeChanges {
    /// Charges beside the plan's, each with one price in the subscription's currency.
    #[serde(default)]
    pub add: Vec<Charge>,
    /// Charges of the subscription at another price.
    #[serde(default)]
    pub edit: Vec<ChargeOverride>,
    /// Charges removed, by product code.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// How an amendment changes the subscription's add-ons.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct AddOnChanges {
    /// Add-ons bought.
    #[serde(default)]
    pub add: Vec<AddOnPurchase>,
    /// Add-ons held at another quantity.
    #[serde(default)]
    pub edit: Vec<AddOnPurchase>,
    /// Add-ons removed, by code.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// Changes to a subscription's charges and add-ons, its plan kept.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = AmendSubscription)]
#[serde(deny_unknown_fields)]
pub struct AmendmentInput {
    /// Charges added, repriced or removed.
    #[serde(default)]
    pub charges: ChargeChanges,
    /// Add-ons added, held at another quantity or removed.
    #[serde(default)]
    pub add_ons: AddOnChanges,
    /// Required: `immediate` or `period_end`.
    #[serde(default)]
    pub when: Option<When>,
}

/// A cancellation of the subscription.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = CancelSubscription)]
#[serde(deny_unknown_fields)]
pub struct CancelInput {
    /// Required: `immediate`, `period_end` or `date` (today or later, not after the
    /// subscription's own end).
    #[serde(default)]
    pub when: Option<When>,
    /// Why it is cancelled, kept with the cancellation; blank reads as none.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Which way a change goes: `downgrade` when any part of it gets less (a cheaper fee, less
/// credit, a shorter cadence, fewer seats, a lower level; a cancellation is one); `upgrade` when
/// a part gets more and none less; `lateral` when none changes either way.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Upgrade,
    Downgrade,
    Lateral,
}

impl From<ChangeDirection> for Direction {
    fn from(direction: ChangeDirection) -> Self {
        match direction {
            ChangeDirection::Upgrade => Direction::Upgrade,
            ChangeDirection::Downgrade => Direction::Downgrade,
            ChangeDirection::Lateral => Direction::Lateral,
        }
    }
}

/// What a document a change issues is: `invoice` the prorated charges, issued when the change
/// applies; `credit_note` the unused time given back, issued when it applies, its amounts
/// negative; `next_invoice` the next renewal, under the change.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentType {
    /// Issued when the change applies: the prorated charges.
    Invoice,
    /// Issued when the change applies: the unused time given back. Its amounts are negative.
    CreditNote,
    /// The next renewal, under the change.
    NextInvoice,
}

/// A document a change issues, its amounts in minor units, its lines as an invoice's read.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Document {
    /// What the document is.
    #[serde(rename = "type")]
    pub kind: DocumentType,
    /// ISO 4217.
    pub currency: String,
    /// The lines' total before discounts, credit and tax, in minor units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtotal: Option<i64>,
    /// What discounts and coupons take off, in minor units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discount: Option<i64>,
    /// The tax, in minor units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax: Option<i64>,
    /// The total, tax included, in minor units.
    pub total: i64,
    /// What is left to pay once the customer's credit applies, in minor units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_due: Option<i64>,
    /// Its lines, as an invoice's read.
    pub lines: Vec<DocumentLine>,
    /// Its tax by rate; absent when it has none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub taxes: Vec<DocumentTax>,
    /// The coupons it applies; absent when none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub coupons: Vec<DocumentCoupon>,
}

/// A part of a preview that could not be priced, left out of it: `invoice` the invoice the change
/// issues; `credit_note` the credit note it issues; `next_invoice` the renewal under it;
/// `subscription_after` the subscription it leaves, which then reads the new charges on the
/// subscription as it stands, its MRR possibly left out.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewPart {
    /// The invoice the change issues.
    Invoice,
    /// The credit note it issues.
    CreditNote,
    /// The renewal under it.
    NextInvoice,
    /// The subscription it leaves: `subscription_after` then reads the new charges on the
    /// subscription as it stands, its MRR possibly left out.
    SubscriptionAfter,
}

/// What a change would do, nothing written.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ChangePreview {
    /// The day it would take effect.
    pub effective_date: NaiveDate,
    /// Which way it goes.
    pub direction: Direction,
    /// When it takes effect, as asked or as its rules decide.
    pub when: WhenType,
    /// The documents it would issue: the invoice and credit note when it applies, the renewal
    /// under it.
    pub documents: Vec<Document>,
    /// True when a part of the preview could not be priced and is left out (`missing` names
    /// it): `documents` then never reads as the change billing nothing.
    pub incomplete: bool,
    /// What is left out, when `incomplete`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<PreviewPart>,
    /// The subscription as it would read once the change is in force: its plan and option,
    /// charges, add-ons, quantities, levels, minimum and MRR.
    pub subscription_after: Subscription,
    /// What the change does to the subscription's timeline (its phases, dated add-ons and
    /// discounts, renewal), when one follows it: each written again over the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<TimelineEffect>,
}

/// A subscription's timeline written again over a change.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct TimelineEffect {
    pub entries: Vec<TimelineEntry>,
    /// The invoices ahead without the change, fixed fees before tax and credits.
    pub invoices_before: Vec<TimelineInvoice>,
    /// The same with the change.
    pub invoices_after: Vec<TimelineInvoice>,
    pub total_before: i64,
    pub total_after: i64,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct TimelineInvoice {
    pub date: NaiveDate,
    /// Minor units.
    pub total: i64,
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimelineOwnerType {
    Phase,
    /// A dated add-on or discount.
    Item,
    Renewal,
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimelineChangeEffect {
    /// Written again as it was.
    Kept,
    /// Some of its terms name nothing on the subscription from its start: kept, billing nothing.
    Inactive,
    /// Its start is no longer a renewal: settled on its day.
    Prorated,
    /// Withdrawn: what it changes is no longer on the subscription.
    Dropped,
    /// Its switch is replaced by this plan change.
    SwitchReplaced,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct TimelineEntry {
    pub owner_type: TimelineOwnerType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    pub label: String,
    pub date: NaiveDate,
    pub effect: TimelineChangeEffect,
    /// The inactive terms, or why it was dropped.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

impl From<meteroid_store::domain::subscription_phases::TimelineEffect> for TimelineEffect {
    fn from(effect: meteroid_store::domain::subscription_phases::TimelineEffect) -> Self {
        use meteroid_store::domain::subscription_phases::{RewriteEffect, TimelineOwner};
        let totals = |projection: &meteroid_store::services::timeline::Projection| {
            projection
                .invoices
                .iter()
                .map(|invoice| TimelineInvoice {
                    date: invoice.on,
                    total: invoice.total,
                })
                .collect()
        };
        TimelineEffect {
            invoices_before: totals(&effect.before),
            invoices_after: totals(&effect.after),
            total_before: effect.before.total,
            total_after: effect.after.total,
            entries: effect
                .entries
                .into_iter()
                .map(|entry| {
                    let (owner_type, owner_id) = match entry.owner {
                        TimelineOwner::Phase(id) => {
                            (TimelineOwnerType::Phase, Some(id.to_string()))
                        }
                        TimelineOwner::Item(id) => (TimelineOwnerType::Item, Some(id.to_string())),
                        TimelineOwner::Renewal => (TimelineOwnerType::Renewal, None),
                    };
                    let (effect, details) = match entry.effect {
                        RewriteEffect::Kept => (TimelineChangeEffect::Kept, Vec::new()),
                        RewriteEffect::Inactive(terms) => (TimelineChangeEffect::Inactive, terms),
                        RewriteEffect::Prorated => (TimelineChangeEffect::Prorated, Vec::new()),
                        RewriteEffect::Dropped(why) => (TimelineChangeEffect::Dropped, vec![why]),
                        RewriteEffect::SwitchReplaced => {
                            (TimelineChangeEffect::SwitchReplaced, Vec::new())
                        }
                    };
                    TimelineEntry {
                        owner_type,
                        owner_id,
                        label: entry.label,
                        date: entry.on,
                        effect,
                        details,
                    }
                })
                .collect(),
        }
    }
}

/// Where a change made stands: `applied` in force; `scheduled` waiting for `effective_date` (see
/// the subscription's `schedule`); `pending_payment` waiting for the payment of
/// `awaiting_payment`, the invoice billing it, and applied once that invoice is paid.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    /// In force.
    Applied,
    /// Waiting for `effective_date`: see the subscription's `schedule`.
    Scheduled,
    /// Waiting for the payment of `awaiting_payment`, the invoice billing it: it applies once
    /// that invoice is paid.
    PendingPayment,
}

/// A change made.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ChangeResult {
    /// Where the change stands.
    pub status: ChangeStatus,
    /// The day it took or takes effect.
    pub effective_date: NaiveDate,
    /// The scheduled change, revocable until its date.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "common_domain::ids::string_serde_opt::serialize")]
    pub scheduled_change: Option<ScheduledEventId>,
    /// Invoices the change issued.
    pub invoices: Vec<InvoiceRef>,
    /// Credit notes the change issued.
    pub credit_notes: Vec<CreditNoteRef>,
    /// The invoice whose payment applies a `pending_payment` change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awaiting_payment: Option<InvoiceRef>,
    /// The subscription once changed; left out when it cannot be read back in v2 (data API v2
    /// does not express, read with API v1): the change is made all the same.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Subscription>,
}
