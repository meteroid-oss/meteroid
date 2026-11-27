//! Trials (ADR 0015 §3, R-153, R-159): one definition, the trial offer. A plan's trial is the
//! offer it names; a subscription's trial is its fact, fixed at its start.
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{PlanId, PlanVersionId, TrialOfferId};
use diesel_models::trial_offers::TrialOfferRow;
use error_stack::Report;

use crate::errors::{StoreError, StoreErrorReport};

/// The longest trial, in days, from its billing start.
pub const MAX_TRIAL_DAYS: u32 = 3650;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrialEndAction {
    /// The subscription goes on, billed from the trial's end.
    Convert,
    /// It ends with the trial.
    Cancel,
    /// It switches to another plan at the trial's end.
    Fallback { plan_version_id: PlanVersionId },
}

impl TrialEndAction {
    pub fn kind(&self) -> TrialEnd {
        match self {
            TrialEndAction::Convert => TrialEnd::Convert,
            TrialEndAction::Cancel => TrialEnd::Cancel,
            TrialEndAction::Fallback { .. } => TrialEnd::Fallback,
        }
    }

    pub fn as_str(&self) -> &'static str {
        self.kind().as_str()
    }
}

/// What a running trial's end does, as its fact records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialEnd {
    Convert,
    /// Its end is the subscription's end date (a COMPLETE ending), no term.
    Cancel,
    /// A phase switching plan starts on its end.
    Fallback,
}

impl TrialEnd {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrialEnd::Convert => "CONVERT",
            TrialEnd::Cancel => "CANCEL",
            TrialEnd::Fallback => "FALLBACK",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "CANCEL" => TrialEnd::Cancel,
            "FALLBACK" => TrialEnd::Fallback,
            _ => TrialEnd::Convert,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialEligibility {
    Offer,
    Plan,
    ProductFamily,
    /// Given without limit.
    None,
}

impl TrialEligibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrialEligibility::Offer => "OFFER",
            TrialEligibility::Plan => "PLAN",
            TrialEligibility::ProductFamily => "PRODUCT_FAMILY",
            TrialEligibility::None => "NONE",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "OFFER" => TrialEligibility::Offer,
            "PLAN" => TrialEligibility::Plan,
            "PRODUCT_FAMILY" => TrialEligibility::ProductFamily,
            "NONE" => TrialEligibility::None,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialOffer {
    pub id: TrialOfferId,
    pub name: String,
    pub code: Option<String>,
    pub duration_days: u32,
    pub free: bool,
    /// The plan whose features the trial gives over the subscription's own (a reverse trial).
    pub trial_plan_id: Option<PlanId>,
    pub end_action: TrialEndAction,
    pub eligibility: TrialEligibility,
    pub archived_at: Option<NaiveDateTime>,
}

impl TryFrom<TrialOfferRow> for TrialOffer {
    type Error = StoreErrorReport;

    fn try_from(row: TrialOfferRow) -> Result<Self, Self::Error> {
        let bad = |what: &str| {
            Report::new(StoreError::InvalidArgument(format!(
                "trial offer {}: {what}",
                row.id
            )))
        };
        let end_action = end_action_of(&row.end_action, row.fallback_plan_version_id)
            .ok_or_else(|| bad("unknown end action"))?;
        let eligibility = TrialEligibility::parse(&row.eligibility_scope)
            .ok_or_else(|| bad("unknown eligibility"))?;
        Ok(TrialOffer {
            id: row.id,
            name: row.name,
            code: row.code,
            duration_days: u32::try_from(row.duration_days).unwrap_or(1),
            free: row.free,
            trial_plan_id: row.trial_plan_id,
            end_action,
            eligibility,
            archived_at: row.archived_at,
        })
    }
}

pub(crate) fn end_action_of(
    value: &str,
    fallback: Option<PlanVersionId>,
) -> Option<TrialEndAction> {
    Some(match (value, fallback) {
        ("CONVERT", _) => TrialEndAction::Convert,
        ("CANCEL", _) => TrialEndAction::Cancel,
        ("FALLBACK", Some(plan_version_id)) => TrialEndAction::Fallback { plan_version_id },
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct TrialOfferNew {
    pub name: String,
    pub code: Option<String>,
    pub duration_days: u32,
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
    pub end_action: TrialEndAction,
    pub eligibility: TrialEligibility,
}

/// An offer's edit, merged field by field: what it names changes, for trials to come only.
#[derive(Debug, Clone, Default)]
pub struct TrialOfferPatch {
    pub name: Option<String>,
    pub code: Option<Option<String>>,
    pub duration_days: Option<u32>,
    pub free: Option<bool>,
    pub trial_plan_id: Option<Option<PlanId>>,
    pub end_action: Option<TrialEndAction>,
    pub eligibility: Option<TrialEligibility>,
}

impl TrialOffer {
    /// The offer as it would be created, `patch` applied.
    pub fn patched(&self, patch: TrialOfferPatch) -> TrialOfferNew {
        TrialOfferNew {
            name: patch.name.unwrap_or_else(|| self.name.clone()),
            code: patch.code.unwrap_or_else(|| self.code.clone()),
            duration_days: patch.duration_days.unwrap_or(self.duration_days),
            free: patch.free.unwrap_or(self.free),
            trial_plan_id: patch.trial_plan_id.unwrap_or(self.trial_plan_id),
            end_action: patch.end_action.unwrap_or_else(|| self.end_action.clone()),
            eligibility: patch.eligibility.unwrap_or(self.eligibility),
        }
    }

    pub fn terms(&self) -> TrialTerms {
        TrialTerms {
            duration_days: self.duration_days,
            free: self.free,
            trial_plan_id: self.trial_plan_id,
        }
    }
}

/// What a plan's trial gives, as the plan editor writes it; the rest of its offer is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialTerms {
    pub duration_days: u32,
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
}

/// A plan's trial written: its terms (`None` removes it), or a catalog offer named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanTrialChange {
    Terms(Option<TrialTerms>),
    Offer(TrialOfferId),
}

/// A plan's trial: the offer it names, and every plan naming it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTrial {
    pub offer: TrialOffer,
    pub plans: Vec<PlanId>,
}

/// How long a trial asked at a creation lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialLength {
    Days(u32),
    /// Up to this day, exclusive.
    Until(NaiveDate),
}

/// The trial a creation asks for (R-153).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TrialRequest {
    /// The plan's trial: what a creation asking nothing gets.
    #[default]
    Plan,
    None,
    Offer(TrialOfferId),
    /// Its own terms; freeness and trial plan the plan's trial's when left out, else free, none.
    Custom {
        length: TrialLength,
        free: Option<bool>,
        trial_plan: Option<Option<PlanId>>,
    },
}

impl TrialRequest {
    /// A request in days as the older APIs give it: none asks the plan's trial, 0 none.
    pub fn from_days(days: Option<u32>, free: Option<bool>) -> Self {
        match days {
            None => TrialRequest::Plan,
            Some(0) => TrialRequest::None,
            Some(days) => TrialRequest::Custom {
                length: TrialLength::Days(days),
                free,
                trial_plan: None,
            },
        }
    }
}

/// The trial a creation gets, resolved once (`trials::resolve_trial_tx`) from its billing start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTrial {
    pub days: u32,
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
    pub offer: Option<TrialOfferId>,
    pub end_action: TrialEndAction,
    /// What the customer is given once, when the offer limits it.
    pub eligibility: TrialEligibility,
}

impl ResolvedTrial {
    pub fn until(&self, billing_start: NaiveDate) -> NaiveDate {
        billing_start + chrono::Duration::days(i64::from(self.days))
    }
}

/// Where a running trial's end moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialMove {
    /// Later, ending on this day (exclusive).
    Until(NaiveDate),
    /// Later by this many days.
    Days(u32),
    /// Today: billing starts now.
    Now,
}

/// A subscription's trial, as its fact records it: from its billing start to `until` (exclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionTrial {
    pub from: NaiveDate,
    pub until: NaiveDate,
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
    pub offer_id: Option<TrialOfferId>,
    pub end_action: TrialEnd,
}

impl SubscriptionTrial {
    pub fn days(&self) -> u32 {
        u32::try_from((self.until - self.from).num_days()).unwrap_or(0)
    }
}

impl From<diesel_models::subscription_facts::SubscriptionTrialRow> for SubscriptionTrial {
    fn from(row: diesel_models::subscription_facts::SubscriptionTrialRow) -> Self {
        SubscriptionTrial {
            from: row.from_date,
            until: row.until_date,
            free: row.free,
            trial_plan_id: row.trial_plan_id,
            offer_id: row.trial_offer_id,
            end_action: TrialEnd::parse(&row.end_action),
        }
    }
}
