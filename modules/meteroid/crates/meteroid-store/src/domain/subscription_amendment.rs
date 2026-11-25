use crate::domain::price_components::{PriceEntry, ProductRef};
use crate::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use crate::domain::subscription_changes::{
    AddedComponent, ChangeDirection, PlanChangeMode, ProrationSummary, RemovedComponent,
};
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use chrono::NaiveDate;
use common_domain::ids::{
    CreditNoteId, InvoiceId, SubscriptionAddOnId, SubscriptionPriceComponentId,
};

/// A manual/sales-led amendment: a batch of changes applied to a live subscription
/// without switching the plan version.
#[derive(Debug, Clone)]
pub struct SubscriptionAmendment {
    pub apply_mode: PlanChangeMode,
    pub component_changes: ComponentChanges,
    pub add_on_changes: AddOnChanges,
}

impl SubscriptionAmendment {
    pub fn is_empty(&self) -> bool {
        self.component_changes.edited.is_empty()
            && self.component_changes.added.is_empty()
            && self.component_changes.removed.is_empty()
            && self.add_on_changes.added.is_empty()
            && self.add_on_changes.edited.is_empty()
            && self.add_on_changes.removed.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ComponentChanges {
    pub edited: Vec<EditComponent>,
    pub added: Vec<AddExtraComponent>,
    pub removed: Vec<SubscriptionPriceComponentId>,
}

/// Override an existing live component's price/fee (keeps the same product).
#[derive(Debug, Clone)]
pub struct EditComponent {
    pub subscription_component_id: SubscriptionPriceComponentId,
    pub name: Option<String>,
    pub price_entry: PriceEntry,
}

/// Add an ad-hoc extra component (no plan price_component reference).
#[derive(Debug, Clone)]
pub struct AddExtraComponent {
    pub name: String,
    pub product: ProductRef,
    pub price_entry: PriceEntry,
    /// How the charge bills the product; None takes a new product's, or an existing price's
    /// charge's.
    pub fee_structure: Option<crate::domain::prices::FeeStructure>,
}

#[derive(Debug, Clone, Default)]
pub struct AddOnChanges {
    pub added: Vec<CreateSubscriptionAddOn>,
    pub edited: Vec<EditSubscriptionAddOn>,
    pub removed: Vec<SubscriptionAddOnId>,
}

/// Change the quantity and/or customization of an existing live add-on.
#[derive(Debug, Clone)]
pub struct EditSubscriptionAddOn {
    pub subscription_add_on_id: SubscriptionAddOnId,
    pub quantity: Option<u32>,
    pub customization: Option<SubscriptionAddOnCustomization>,
}

/// Preview of an amendment. Edits (component overrides, add-on quantity/customization
/// changes) are decomposed into a removed (old) + added (new) pair, which is
/// proration-equivalent to a matched credit+charge and keeps a clean temporal cut.
#[derive(Debug, Clone)]
pub struct AmendmentPreview {
    pub component_added: Vec<AddedComponent>,
    pub component_removed: Vec<RemovedComponent>,
    pub addon_added: Vec<AddedComponent>,
    pub addon_removed: Vec<RemovedComponent>,
    pub effective_date: NaiveDate,
}

impl AmendmentPreview {
    /// All added lines (components + add-ons) for proration/direction.
    pub fn all_added(&self) -> Vec<AddedComponent> {
        self.component_added
            .iter()
            .chain(self.addon_added.iter())
            .cloned()
            .collect()
    }

    /// All removed lines (components + add-ons) for proration/direction.
    pub fn all_removed(&self) -> Vec<RemovedComponent> {
        self.component_removed
            .iter()
            .chain(self.addon_removed.iter())
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct AmendmentPreviewExtended {
    pub preview: AmendmentPreview,
    pub proration: Option<ProrationSummary>,
    pub change_direction: ChangeDirection,
    pub mrr_before_cents: i64,
    pub mrr_after_cents: i64,
    /// The prorated adjustment invoice issued now (immediate, non-trial).
    pub adjustment_invoice: Option<ComputedInvoiceContent>,
    /// The credit note issued now for the credit side of an immediate amendment
    /// (downgrades/removals). Lines carry negative amounts.
    pub credit_note: Option<ComputedInvoiceContent>,
    /// The next renewal invoice under the amended subscription.
    pub next_invoice: Option<ComputedInvoiceContent>,
    /// Dated on the change, up to where its last prorated stub ends.
    pub adjustment_period: Option<crate::domain::Period>,
    /// The subscription as the amendment leaves it, the next invoice billing its period's end.
    pub subscription_after: crate::domain::Subscription,
}

#[derive(Debug, Clone)]
pub struct ImmediateAmendmentResult {
    pub adjustment_invoice_id: Option<InvoiceId>,
    /// Credit notes issued for the credit side of the amendment — one per source
    /// invoice that billed a now-removed/downgraded item (the period's recurring
    /// invoice and/or in-period adjustment invoices). Empty when nothing is owed.
    pub credit_note_ids: Vec<CreditNoteId>,
    pub effective_date: NaiveDate,
}

/// A level of a usage-levels charge: one of the capacity prices of its plan component's cell, in
/// the subscription's option, currency and cadence (R-135).
#[derive(Debug, Clone)]
pub struct CapacityLevel {
    pub price_id: common_domain::ids::PriceId,
    pub included: u64,
    pub rate: rust_decimal::Decimal,
    pub overage_rate: rust_decimal::Decimal,
    /// The level the subscription bills now.
    pub current: bool,
}

/// A subscription's usage-levels charge, with the levels it may move to, fewest units first.
#[derive(Debug, Clone)]
pub struct ComponentLevels {
    pub subscription_component_id: SubscriptionPriceComponentId,
    pub price_component_id: common_domain::ids::PriceComponentId,
    pub name: String,
    pub period: crate::domain::enums::SubscriptionFeeBillingPeriod,
    /// The units it includes now.
    pub included: u64,
    pub levels: Vec<CapacityLevel>,
}

/// A move to another level, as it applies: a higher level at once, a lower one at the period's
/// end (R-135).
#[derive(Debug, Clone)]
pub struct LevelChangePreview {
    pub direction: ChangeDirection,
    pub apply_mode: PlanChangeMode,
    pub effective_date: NaiveDate,
    pub from_included: u64,
    pub to_included: u64,
    pub preview: AmendmentPreviewExtended,
}

#[derive(Debug, Clone)]
pub enum LevelChangeOutcome {
    Applied(ImmediateAmendmentResult),
    Scheduled(Box<crate::domain::scheduled_events::ScheduledEvent>),
}
