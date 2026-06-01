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
