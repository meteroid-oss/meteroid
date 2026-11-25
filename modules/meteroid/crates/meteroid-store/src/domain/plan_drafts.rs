//! A plan version's draft written whole, as API v2's `PUT /plans/{id}/draft` sends it (ADR-0013):
//! its options by name and alias, each charge's prices with the options they are in, and its other
//! terms. Written in one transaction by `PlanDraftsInterface`, replacing the draft there was.
use super::plan_minimums::MinimumScope;
use super::plans::PlanVersionNewInternal;
use super::price_components::PriceInput;
use super::prices::FeeStructure;
use common_domain::ids::{AddOnId, ProductId};

/// An option of the draft; the first is the default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDraftOption {
    pub name: String,
    pub alias: String,
    /// Sold when named, never advertised; the default option never is.
    pub hidden: bool,
}

/// The options of the draft a price is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftPriceOptions {
    Every,
    /// By alias, each an option of the draft.
    Only(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct DraftPrice {
    pub price: PriceInput,
    pub options: DraftPriceOptions,
}

/// A charge of the draft: an existing product billed as `fee_structure` says; no structure and no
/// price includes the product.
#[derive(Debug, Clone)]
pub struct DraftCharge {
    /// Unique within the version.
    pub name: String,
    pub product_id: ProductId,
    pub fee_structure: Option<FeeStructure>,
    pub prices: Vec<DraftPrice>,
}

#[derive(Debug, Clone)]
pub struct DraftAddOn {
    pub add_on_id: AddOnId,
    pub self_serviceable: Option<bool>,
    pub max_instances_per_subscription: Option<i32>,
}

/// The floor a version commits to, in minor units of the version's currency.
#[derive(Debug, Clone)]
pub struct DraftMinimum {
    pub amount_subunit: i64,
    pub scope: MinimumScope,
}

#[derive(Debug, Clone)]
pub struct PlanDraft {
    /// The version's terms; `is_draft_version` is ignored, a draft is written.
    pub version: PlanVersionNewInternal,
    /// At least one; the first is the default and is shown.
    pub options: Vec<PlanDraftOption>,
    pub charges: Vec<DraftCharge>,
    pub add_ons: Vec<DraftAddOn>,
    pub minimum: Option<DraftMinimum>,
}
