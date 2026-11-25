//! The subscription resources of API v2 (ADR-0013 §10): what a subscription reads as, what
//! creating one and changing one take, and what a change previews and answers.
use crate::v2::charges::{Cadence, Charge, PriceModel};
use crate::v2::documents::{DocumentCoupon, DocumentLine, DocumentTax};
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

/// What starts the subscription.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<ApiDateTime>,
    /// When it ended (cancelled, completed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<ApiDateTime>,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CurrentPeriod {
    pub start: NaiveDate,
    /// Exclusive; none for a subscription not billed by period yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct TrialState {
    pub days: u32,
    /// Nothing is billed during it; read on a single subscription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free: Option<bool>,
    /// When it ended or ends; read on a single subscription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_on: Option<NaiveDate>,
}

/// A count to come: a decrease waiting for the period's end, an increase waiting for its payment
/// or for the next period.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct PendingQuantity {
    /// The count once it applies.
    #[schema(value_type = String, format = "decimal", example = "10")]
    pub value: Decimal,
    pub effective_at: ApiDateTime,
}

/// A managed quantity the subscription holds (seats).
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct QuantityState {
    #[schema(value_type = String, format = "decimal", example = "12")]
    pub held: Decimal,
    pub pending: Vec<PendingQuantity>,
}

/// Where a charge of the subscription comes from.
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
    pub origin: ChargeOrigin,
    /// The add-on of a charge from an add-on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_on: Option<AddOnRef>,
    #[serde(flatten)]
    pub charge: Charge,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubscribedAddOn {
    pub add_on: AddOnRef,
    pub name: String,
    pub quantity: i32,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct AppliedCoupon {
    pub coupon: CouponRef,
    /// Still discounting.
    pub active: bool,
    /// Invoices it discounted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_count: Option<i32>,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledChangeType {
    PlanChange,
    Cancellation,
    Amendment,
}

/// The names of what a scheduled amendment adds and removes.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Default)]
pub struct AmendmentSummary {
    pub added_charges: Vec<String>,
    pub removed_charges: Vec<String>,
    pub added_add_ons: Vec<String>,
    pub removed_add_ons: Vec<String>,
}

/// A change waiting for its date.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct ScheduledChange {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ScheduledEventId,
    #[serde(rename = "type")]
    pub kind: ScheduledChangeType,
    pub effective_date: NaiveDate,
    /// The plan a plan change moves to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanRef>,
    /// A cancellation's reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub initiated_by_customer: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amendment: Option<AmendmentSummary>,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Schedule {
    /// Soonest first.
    pub changes: Vec<ScheduledChange>,
}

/// A cancellation waiting for its date: revoke it with `POST …/cancel/revoke`.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Cancellation {
    pub effective_date: NaiveDate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub initiated_by_customer: bool,
}

/// An amount in minor units of its currency.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct Money {
    pub amount: i64,
    pub currency: String,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Invoicing {
    /// Days an invoice is due after it is issued.
    pub net_terms: u32,
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

/// Restricts the customer's usage events the subscription bills to those whose `property` is
/// `value` (an API key, a project); one without bills the customer's events no other
/// subscription's filter takes.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct UsageFilter {
    pub property: String,
    pub value: String,
    /// When it started to apply: the start of the usage not invoiced yet when it was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_from: Option<ApiDateTime>,
}

/// A subscription. Lists carry what the subscription itself holds; `quantities`, `levels`,
/// `charges`, `add_ons`, `coupons`, `schedule`, `cancellation` and `minimum` are read on a single
/// one.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Subscription {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: SubscriptionId,
    /// The integrator's own id, unique per tenant; paths take it in place of the id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub customer: CustomerRef,
    pub plan: PlanRef,
    pub status: SubscriptionStatus,
    pub currency: String,
    /// How often it renews.
    pub period: Cadence,
    pub activation: Activation,
    /// The day of the month its periods start on.
    pub billing_day_anchor: u16,
    pub dates: Dates,
    pub current_period: CurrentPeriod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<TrialState>,
    /// The managed quantities it holds (seats), by code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantities: Option<BTreeMap<String, QuantityState>>,
    /// The level of each usage-levels charge, by product code, as the usage it includes:
    /// `{"api": "10000"}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charges: Option<Vec<SubscriptionCharge>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_ons: Option<Vec<SubscribedAddOn>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupons: Option<Vec<AppliedCoupon>>,
    /// The changes waiting for their date, soonest first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<ScheduledChange>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<Cancellation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<Minimum>,
    /// Monthly recurring revenue, tax excluded.
    pub mrr: Money,
    /// Its prices are quoted tax included (its plan version's, when it was created).
    pub tax_inclusive: bool,
    /// The customer's events it bills, when it does not bill all of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_filter: Option<UsageFilter>,
    pub invoicing: Invoicing,
    #[schema(value_type = HashMap<String, Value>)]
    pub custom_properties: serde_json::Value,
    pub created_at: ApiDateTime,
    /// When it, or anything read of it, last changed.
    pub updated_at: ApiDateTime,
}

crate::v2_list!(SubscriptionList, Subscription);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct SubscriptionListQuery {
    /// Only this customer's: its id, or `ext:<alias>`.
    pub customer: Option<String>,
    /// Only those on this plan, by code or id.
    pub plan: Option<String>,
    pub status: Option<SubscriptionStatus>,
    /// Only those changed after this moment (RFC 3339).
    #[param(format = DateTime)]
    pub updated_after: Option<String>,
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
    /// The plan's when left out (due on receipt when it sells credit).
    #[serde(default)]
    pub net_terms: Option<u32>,
    #[serde(default)]
    pub purchase_order: Option<String>,
    #[serde(default)]
    pub memo: Option<String>,
    /// True when left out.
    #[serde(default)]
    pub auto_finalize: Option<bool>,
    /// True when left out: the invoicing entity then needs a payment provider.
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
    pub price: PriceModel,
    /// The charge's cadence when left out.
    #[serde(default)]
    pub cadence: Option<Cadence>,
}

/// How a new subscription's charges differ from its plan's.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct CreateCharges {
    /// Plan charges at another price.
    #[serde(default, rename = "override")]
    pub overrides: Vec<ChargeOverride>,
    /// Charges beside the plan's, each with one price in the subscription's currency.
    #[serde(default)]
    pub add: Vec<Charge>,
    /// Plan charges left out, by product code.
    #[serde(default)]
    pub remove: Vec<String>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AddOnPurchase {
    /// By code or id.
    pub add_on: String,
    /// Instances held; 1 when left out.
    #[serde(default = "one")]
    pub quantity: u32,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateSubscription {
    /// The customer's id, or its alias as `ext:<alias>`.
    pub customer: String,
    /// By code or id.
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
    /// The integrator's own id, unique per tenant.
    #[serde(default)]
    pub external_id: Option<String>,
    /// Today when left out; a past date bills the periods since.
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    /// The plan's trial when left out.
    #[serde(default)]
    pub trial_days: Option<u32>,
    /// The day of the month periods start on; the start date's when left out.
    #[serde(default)]
    pub billing_day_anchor: Option<u16>,
    #[serde(default)]
    pub activation: Activation,
    /// The starting count of each managed quantity its seat charges count, by code: `{"users": "12"}`.
    #[serde(default)]
    pub quantities: BTreeMap<String, String>,
    /// The level of each usage-levels charge, by product code, as the usage it includes:
    /// `{"api": "10000"}`. The lowest when left out.
    #[serde(default)]
    pub levels: BTreeMap<String, String>,
    #[serde(default)]
    pub charges: CreateCharges,
    #[serde(default)]
    pub add_ons: Vec<AddOnPurchase>,
    /// Coupon codes.
    #[serde(default)]
    pub coupons: Vec<String>,
    #[serde(default)]
    pub invoicing: InvoicingInput,
    /// Validated against the tenant's custom property definitions.
    #[serde(default)]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

// ── Changing ────────────────────────────────────────────────────────

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
    #[serde(rename = "type")]
    pub kind: WhenType,
    /// With `type: date` only.
    #[serde(default)]
    pub date: Option<NaiveDate>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = ChangePlan)]
#[serde(deny_unknown_fields)]
pub struct PlanChangeInput {
    /// By code or id.
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
    /// Required: `immediate` or `period_end`; a downgrade at the period's end only.
    #[serde(default)]
    pub when: Option<When>,
}

/// When the seats an increase adds are in force.
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

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = ChangeLevel)]
#[serde(deny_unknown_fields)]
pub struct LevelChangeInput {
    /// The usage the level includes, one of the charge's levels.
    #[schema(format = "decimal", example = "10000")]
    pub included: String,
    /// A higher level applies `immediate`, a lower one at `period_end`; checked when sent.
    #[serde(default)]
    pub when: Option<When>,
}

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

#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct AddOnChanges {
    #[serde(default)]
    pub add: Vec<AddOnPurchase>,
    /// Add-ons held at another quantity.
    #[serde(default)]
    pub edit: Vec<AddOnPurchase>,
    /// By add-on code.
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = AmendSubscription)]
#[serde(deny_unknown_fields)]
pub struct AmendmentInput {
    #[serde(default)]
    pub charges: ChargeChanges,
    #[serde(default)]
    pub add_ons: AddOnChanges,
    /// Required: `immediate` or `period_end`.
    #[serde(default)]
    pub when: Option<When>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[schema(as = CancelSubscription)]
#[serde(deny_unknown_fields)]
pub struct CancelInput {
    /// Required: `immediate`, `period_end` or `date`.
    #[serde(default)]
    pub when: Option<When>,
    #[serde(default)]
    pub reason: Option<String>,
}

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
    #[serde(rename = "type")]
    pub kind: DocumentType,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtotal: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discount: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax: Option<i64>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_due: Option<i64>,
    pub lines: Vec<DocumentLine>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub taxes: Vec<DocumentTax>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub coupons: Vec<DocumentCoupon>,
}

/// A part of a preview that could not be priced, left out of it.
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
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct ChangePreview {
    pub effective_date: NaiveDate,
    pub direction: Direction,
    /// When it takes effect, as asked or as its rules decide.
    pub when: WhenType,
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
}

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
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct ChangeResult {
    pub status: ChangeStatus,
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
