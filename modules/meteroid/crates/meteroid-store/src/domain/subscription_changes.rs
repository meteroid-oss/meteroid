use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::subscription_components::SubscriptionFee;
use chrono::NaiveDate;
use common_domain::ids::{
    CreditNoteId, InvoiceId, PriceComponentId, ProductId, SubscriptionAddOnId,
    SubscriptionPriceComponentId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanChangeMode {
    Immediate,
    EndOfPeriod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeDirection {
    Upgrade,
    Downgrade,
    Lateral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChangePreview {
    pub matched: Vec<MatchedComponent>,
    pub added: Vec<AddedComponent>,
    pub removed: Vec<RemovedComponent>,
    pub effective_date: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChangePreviewExtended {
    pub preview: PlanChangePreview,
    pub proration: Option<ProrationSummary>,
    pub change_direction: ChangeDirection,
    /// False when a minimum commitment holds the change to the period's end.
    pub immediate_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProrationSummary {
    pub credits_total_cents: i64,
    pub charges_total_cents: i64,
    pub net_amount_cents: i64,
    /// The credit that would actually be issued as a credit note: the sum of the
    /// post-netting negative lines. For a price override the credit and charge net
    /// into a single line, so an upgrade contributes 0 here even though
    /// `credits_total_cents` (gross) is negative. Use this — not the gross credit —
    /// to decide whether a credit is genuinely owed.
    #[serde(default)]
    pub net_credit_cents: i64,
    /// Prorated arrears charge for newly-added arrears-billing components. These
    /// are NOT billed on the immediate adjustment invoice — they land on the next
    /// renewal invoice. Kept separate so the summary's charges/net match the
    /// adjustment invoice exactly, while the UI can still communicate the
    /// deferred amount to the user.
    #[serde(default)]
    pub arrears_charge_cents: i64,
    pub proration_factor: f64,
    pub days_remaining: u32,
    pub days_in_period: u32,
    /// What the change does to credit packs: new packs billed, old packs' credit removed and
    /// the fee credited back for it.
    #[serde(default)]
    pub pack_changes: Vec<PackChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackChangeKind {
    /// A new pack billed on the adjustment invoice, with the credit it grants.
    Charge,
    /// An old pack's unused credit for the period, removed.
    CreditRemoved,
    /// The fee credited back for an old pack, never above what its unused paid credit is worth.
    FeeCredit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackChange {
    pub name: String,
    pub kind: PackChangeKind,
    /// Money, in the subscription currency's minor units; negative for a fee credit.
    pub amount_cents: i64,
    /// Credit granted or removed, in minor units of `credit_unit` (or of the currency).
    pub credit_minor: i64,
    pub credit_unit: Option<crate::domain::credit_units::CreditUnitRef>,
    /// Decimal places of `credit_minor`: the unit's, or the currency's.
    pub credit_precision: u8,
    pub promotional: bool,
}

#[derive(Debug, Clone)]
pub struct ImmediatePlanChangeResult {
    pub adjustment_invoice_id: Option<InvoiceId>,
    /// Notes crediting the old plan's unused fees against the invoices that billed them.
    pub credit_note_ids: Vec<CreditNoteId>,
    /// First invoice created when plan change ends a free trial.
    pub first_invoice_id: Option<InvoiceId>,
    pub effective_date: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedComponent {
    pub product_id: ProductId,
    pub current_name: String,
    pub current_fee: SubscriptionFee,
    pub current_period: SubscriptionFeeBillingPeriod,
    pub new_name: String,
    pub new_fee: SubscriptionFee,
    pub new_period: SubscriptionFeeBillingPeriod,
    /// Lineage root of the current component, matching the lines that billed it.
    #[serde(default)]
    pub current_net_key: Option<String>,
    /// Id the new component is inserted with, stamped on the line billing it.
    #[serde(default)]
    pub billed_component_id: Option<SubscriptionPriceComponentId>,
    #[serde(default)]
    pub price_component_id: Option<PriceComponentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddedComponent {
    pub name: String,
    pub fee: SubscriptionFee,
    pub period: SubscriptionFeeBillingPeriod,
    /// Correlation key tying an override's new charge to the matching old
    /// credit (e.g. the subscription component/add-on id being edited), so the
    /// adjustment invoice can net them and tax only the delta. `None` for
    /// genuinely new components, which are not netted.
    #[serde(default)]
    pub net_key: Option<String>,
    /// For a genuinely-added component applied immediately, the pre-generated
    /// subscription-component id it will be inserted with. Carried onto the
    /// adjustment-invoice line (as `sub_component_id`) so a later removal can
    /// credit the prorated unused portion against that line. `None` for
    /// overrides and add-on adds.
    #[serde(default)]
    pub billed_component_id: Option<SubscriptionPriceComponentId>,
    /// For a genuinely-added add-on applied immediately, the pre-generated
    /// subscription-add-on id it will be inserted with. The add-on analogue of
    /// `billed_component_id`.
    #[serde(default)]
    pub billed_add_on_id: Option<SubscriptionAddOnId>,
    /// For add-ons, the number of instances. Carried through to the proration
    /// line so the adjustment invoice can display qty × unit_price rather than
    /// 1 × total. `None` for price-component adds (which have no instance count).
    #[serde(default)]
    pub instance_quantity: Option<rust_decimal::Decimal>,
    #[serde(default)]
    pub price_component_id: Option<PriceComponentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovedComponent {
    pub name: String,
    pub current_fee: SubscriptionFee,
    pub current_period: SubscriptionFeeBillingPeriod,
    /// See `AddedComponent::net_key`.
    #[serde(default)]
    pub net_key: Option<String>,
}

/// Proration result for a plan change — contains individual line items.
#[derive(Debug, Clone)]
pub struct ProrationResult {
    pub lines: Vec<ProrationLineItem>,
    pub net_amount_cents: i64,
    pub change_date: NaiveDate,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    /// Display only: amounts are prorated by the exact day ratio in Decimal.
    pub proration_factor: f64,
}

#[derive(Debug, Clone)]
pub struct ProrationLineItem {
    pub name: String,
    pub amount_cents: i64,
    pub full_period_amount_cents: i64,
    pub is_credit: bool,
    pub is_prorated: bool,
    /// Billed quantity for display, when meaningful (e.g. a one-time charge of
    /// N units, or an add-on with N instances). The amount stays the line total.
    /// `None` leaves the invoice line to fall back to a single unit.
    pub quantity: Option<rust_decimal::Decimal>,
    /// Per-unit price for display, when known (the original fee rate). Preferred
    /// over deriving `amount / quantity`, which can yield a long repeating decimal
    /// when the amount isn't evenly divisible by the quantity. `None` falls back to
    /// the derived value. The line total is always `amount_cents`, not this × quantity.
    pub unit_price: Option<rust_decimal::Decimal>,
    pub product_id: Option<ProductId>,
    pub price_component_id: Option<PriceComponentId>,
    /// Correlation key for netting override credit/charge pairs in the
    /// adjustment invoice. See `AddedComponent::net_key`.
    pub net_key: Option<String>,
    /// Subscription-component id this line bills (genuine immediate adds only),
    /// stamped onto the adjustment-invoice line so a later removal can match and
    /// credit it. See `AddedComponent::billed_component_id`.
    pub sub_component_id: Option<SubscriptionPriceComponentId>,
    /// Subscription-add-on id this line bills (genuine immediate adds only).
    /// See `AddedComponent::billed_add_on_id`.
    pub sub_add_on_id: Option<SubscriptionAddOnId>,
    /// Revenue taxonomy resolved from the originating fee variant, carried onto the
    /// adjustment-invoice line so analytics classifies it correctly.
    pub charge_type: Option<crate::domain::ChargeType>,
    /// A credit pack's line: the credit it grants once billed.
    pub pack: Option<PackGrant>,
    /// Where the prorated stub this line bills ends, when not at the proration's period end:
    /// a fee on another cadence than the subscription's runs to the end of its own period.
    pub period_end: Option<NaiveDate>,
}

/// The credit a pack line sells, frozen on its invoice line like a period invoice's pack line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackGrant {
    pub credit_minor: i64,
    pub unit: Option<crate::domain::credit_units::CreditUnitRef>,
    pub expiry: Option<crate::domain::price_components::CreditExpiry>,
    pub included: bool,
    pub rollover: Option<crate::domain::credit::GrantRollover>,
}
