//! A subscription's phases: dated stretches with their own terms (a ramp's years). A phase is
//! authoring only: its boundary is a change whose facts are the rows billing reads (R-146).
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    AddOnId, PlanVersionId, PriceComponentId, ProductId, ScheduledEventId, SubscriptionAddOnId,
    SubscriptionId, SubscriptionPhaseId, SubscriptionPriceComponentId,
};
use diesel_models::subscription_phases::{SubscriptionPhaseRow, SubscriptionTimelineItemRow};
use serde::{Deserialize, Serialize};

use crate::domain::scheduled_events::PhaseSeats;
use crate::domain::subscription_components::{ComponentParameterization, SubscriptionFee};
use crate::errors::{StoreError, StoreErrorReport};

/// What a phase bills from its start, absolute: never a delta on the phase before it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhaseTerms {
    /// The fees it prices differently, by the lineage of the component they price.
    #[serde(default)]
    pub components: Vec<PhaseComponent>,
    /// The least seats of each unit billed from its start: a floor on the seats held, which
    /// stay as bought (R-146).
    #[serde(default)]
    pub min_seats: Vec<PhaseSeats>,
    /// The minimum commitment from its start: the one in force kept, ended, or another.
    #[serde(default)]
    pub minimum: PhaseMinimum,
    /// The plan switched to from its start, its cadence with it (R-150).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PhasePlan>,
    /// Starts inside a period: settled on its day as an immediate amendment would be (R-151).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prorated: bool,
}

/// A switch of plan at a phase's start: the fees mapped as a scheduled plan change maps them,
/// then priced by the phase's components and seats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhasePlan {
    pub plan_version_id: PlanVersionId,
    /// The option switched to, by alias; none: the subscription's alias, else the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    #[serde(default)]
    pub component_params: Vec<ComponentParameterization>,
    /// The plan's name, as a quote's signers read it; set when the quote's phases are saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseMinimum {
    #[default]
    Kept,
    Ended,
    /// `amount_subunit` a period, on `products` (every component when empty).
    Amount {
        amount_subunit: i64,
        #[serde(default)]
        products: Vec<ProductId>,
    },
}

/// A fee priced by one amount from the phase's start, its other terms those of the row in force.
/// Named by its lineage, or by its plan component for a fee a switch opens (R-150).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseComponent {
    #[serde(default)]
    pub lineage: Option<SubscriptionPriceComponentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_component: Option<PriceComponentId>,
    pub price: rust_decimal::Decimal,
    /// A template's share of the fee's list price in the subscription's currency, which `price`
    /// then gets at its creation; a template of a plan sold in several currencies prices so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_rate: Option<rust_decimal::Decimal>,
    /// The fee's name when the phase was set, for display.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionPhaseInput {
    pub start_date: NaiveDate,
    pub label: Option<String>,
    pub terms: PhaseTerms,
}

#[derive(Debug, Clone)]
pub struct SubscriptionPhase {
    pub id: SubscriptionPhaseId,
    pub subscription_id: SubscriptionId,
    pub seq: u32,
    pub start_date: NaiveDate,
    /// The next phase's start; open for the last.
    pub end_date: Option<NaiveDate>,
    pub label: Option<String>,
    pub terms: PhaseTerms,
    pub change_id: ScheduledEventId,
    pub created_at: NaiveDateTime,
}

impl TryFrom<SubscriptionPhaseRow> for SubscriptionPhase {
    type Error = StoreErrorReport;

    fn try_from(row: SubscriptionPhaseRow) -> Result<Self, Self::Error> {
        let terms: PhaseTerms = serde_json::from_value(row.terms).map_err(|error| {
            error_stack::Report::new(StoreError::SerdeError("phase terms".into(), error))
        })?;
        Ok(SubscriptionPhase {
            id: row.id,
            subscription_id: row.subscription_id,
            seq: u32::try_from(row.seq).unwrap_or_default(),
            start_date: row.start_date,
            end_date: None,
            label: row.label,
            terms,
            change_id: row.change_id,
            created_at: row.created_at,
        })
    }
}

/// Phases in `seq` order, each ending where the next starts.
pub fn chain(mut phases: Vec<SubscriptionPhase>) -> Vec<SubscriptionPhase> {
    phases.sort_by_key(|phase| phase.seq);
    let starts: Vec<NaiveDate> = phases.iter().map(|phase| phase.start_date).collect();
    for (i, phase) in phases.iter_mut().enumerate() {
        phase.end_date = starts.get(i + 1).copied();
    }
    phases
}

/// The fee at `price`: a rate, a seat's or a quantity's per-unit rate, or a capacity's base rate
/// replaced, the rest kept. None for a fee a phase does not price by one amount (tiers, packages).
pub fn repriced(fee: &SubscriptionFee, price: rust_decimal::Decimal) -> Option<SubscriptionFee> {
    use crate::domain::price_components::RatingModel;
    let mut fee = fee.clone();
    match &mut fee {
        SubscriptionFee::Rate { rate }
        | SubscriptionFee::Recurring {
            rate, model: None, ..
        }
        | SubscriptionFee::Capacity { rate, .. } => *rate = price,
        SubscriptionFee::Recurring {
            model: Some(RatingModel::PerUnit { rate }),
            ..
        }
        | SubscriptionFee::Slot {
            model: RatingModel::PerUnit { rate },
            ..
        } => *rate = price,
        _ => return None,
    }
    Some(fee)
}

/// The one amount a phase prices `fee` by, as [`repriced`] sets it.
pub fn price_of(fee: &SubscriptionFee) -> Option<rust_decimal::Decimal> {
    match fee {
        SubscriptionFee::Rate { rate }
        | SubscriptionFee::Recurring {
            rate, model: None, ..
        }
        | SubscriptionFee::Capacity { rate, .. } => Some(*rate),
        SubscriptionFee::Recurring {
            model: Some(model), ..
        }
        | SubscriptionFee::Slot { model, .. } => model.per_unit_rate(),
        _ => None,
    }
}

/// What a dated add-on bills: free over its window or at its catalog price, then reverting
/// (ending) or converting to paid on the same lineage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineItemTerms {
    #[serde(default)]
    pub free: bool,
    pub end: ItemEnd,
    /// Its boundaries may fall inside a period, each settled on its day (R-151).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prorated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ItemEnd {
    Revert,
    /// Paid from its end: at `price` when set, else the catalog's.
    Convert {
        #[serde(default)]
        price: Option<rust_decimal::Decimal>,
    },
}

/// A new dated add-on, or `existing` given an end.
#[derive(Debug, Clone)]
pub struct TimelineItemInput {
    pub add_on_id: AddOnId,
    pub existing: Option<SubscriptionAddOnId>,
    pub quantity: i32,
    pub start_date: NaiveDate,
    pub until: NaiveDate,
    pub terms: TimelineItemTerms,
    pub label: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TimelineItem {
    pub id: uuid::Uuid,
    pub subscription_id: SubscriptionId,
    pub add_on_id: AddOnId,
    pub lineage: SubscriptionAddOnId,
    pub start_date: NaiveDate,
    pub until: NaiveDate,
    pub terms: TimelineItemTerms,
    pub label: Option<String>,
    pub reason: Option<String>,
    pub start_change_id: Option<ScheduledEventId>,
    pub end_change_id: ScheduledEventId,
}

impl TryFrom<SubscriptionTimelineItemRow> for TimelineItem {
    type Error = StoreErrorReport;

    fn try_from(row: SubscriptionTimelineItemRow) -> Result<Self, Self::Error> {
        let terms: TimelineItemTerms = serde_json::from_value(row.terms).map_err(|error| {
            error_stack::Report::new(StoreError::SerdeError("timeline item terms".into(), error))
        })?;
        let add_on_id = row.add_on_id.ok_or_else(|| {
            error_stack::Report::new(StoreError::InvalidArgument(format!(
                "timeline item {} is not an add-on",
                row.id
            )))
        })?;
        Ok(TimelineItem {
            id: row.id,
            subscription_id: row.subscription_id,
            add_on_id,
            lineage: SubscriptionAddOnId::from(row.lineage_id),
            start_date: row.start_date,
            until: row.until_date,
            terms,
            label: row.label,
            reason: row.reason,
            start_change_id: row.start_change_id,
            end_change_id: row.end_change_id,
        })
    }
}

pub const ITEM_ADD_ON: &str = "ADD_ON";
pub const ITEM_DISCOUNT: &str = "DISCOUNT";

/// A fee billed `percent` off over a window, its price before coming back at the end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscountTerms {
    pub percent: rust_decimal::Decimal,
    /// The fee's name when the discount was set, for display.
    #[serde(default)]
    pub name: Option<String>,
    /// Its boundaries may fall inside a period, each settled on its day (R-151).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prorated: bool,
}

#[derive(Debug, Clone)]
pub struct DatedDiscountInput {
    /// Any row of the fee's lineage.
    pub component: SubscriptionPriceComponentId,
    pub percent: rust_decimal::Decimal,
    pub start_date: NaiveDate,
    pub until: NaiveDate,
    pub prorated: bool,
    pub label: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DatedDiscount {
    pub id: uuid::Uuid,
    pub subscription_id: SubscriptionId,
    pub lineage: SubscriptionPriceComponentId,
    pub start_date: NaiveDate,
    pub until: NaiveDate,
    pub terms: DiscountTerms,
    pub label: Option<String>,
    pub reason: Option<String>,
    pub start_change_id: Option<ScheduledEventId>,
    pub end_change_id: ScheduledEventId,
}

impl TryFrom<SubscriptionTimelineItemRow> for DatedDiscount {
    type Error = StoreErrorReport;

    fn try_from(row: SubscriptionTimelineItemRow) -> Result<Self, Self::Error> {
        let terms: DiscountTerms = serde_json::from_value(row.terms).map_err(|error| {
            error_stack::Report::new(StoreError::SerdeError("discount terms".into(), error))
        })?;
        Ok(DatedDiscount {
            id: row.id,
            subscription_id: row.subscription_id,
            lineage: SubscriptionPriceComponentId::from(row.lineage_id),
            start_date: row.start_date,
            until: row.until_date,
            terms,
            label: row.label,
            reason: row.reason,
            start_change_id: row.start_change_id,
            end_change_id: row.end_change_id,
        })
    }
}

/// A subscription's contract term: where it ends and whether it renews (R-152).
#[derive(Debug, Clone)]
pub struct SubscriptionTermInput {
    pub term_end: NaiveDate,
    pub auto_renew: bool,
    pub renewal_months: u32,
    /// Percent added to each fee priced by one amount at each renewal.
    pub uplift_percent: Option<rust_decimal::Decimal>,
    pub notice_days: u32,
}

#[derive(Debug, Clone)]
pub struct SubscriptionTerm {
    pub subscription_id: SubscriptionId,
    /// The current term's last day: rolled on by each renewal.
    pub term_end: NaiveDate,
    pub auto_renew: bool,
    pub renewal_months: u32,
    pub uplift_percent: Option<rust_decimal::Decimal>,
    pub notice_days: u32,
    /// The last day an opt-out of the renewal is taken.
    pub notice_cutoff: NaiveDate,
    pub renewal_phase_id: Option<SubscriptionPhaseId>,
}

/// One edit of a batch applied to a subscription's timeline in one transaction (R-156).
#[derive(Debug, Clone)]
pub enum TimelineOp {
    SetPhases(Vec<SubscriptionPhaseInput>),
    AddItem(TimelineItemInput),
    AddDiscount(DatedDiscountInput),
    RemoveItem(uuid::Uuid),
    SetTerm(SubscriptionTermInput),
    /// An opt-out taken after the notice cutoff, the operator waiving the notice (R-152).
    SetTermWaivingNotice(SubscriptionTermInput, crate::domain::renewals::NoticeWaiver),
}

/// What a rewrite of the timeline over a change did to one of its owners (R-162).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteEffect {
    /// Written again as it was.
    Kept,
    /// Written again, these terms naming nothing on the subscription on its start: kept, billing
    /// nothing until the change is withdrawn.
    Inactive(Vec<String>),
    /// Written again inside a period, its start no longer a renewal (R-151).
    Prorated,
    /// Withdrawn: what it changes is no longer on the subscription.
    Dropped(String),
    /// Its switch replaced by the plan change of its day.
    SwitchReplaced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineOwner {
    Phase(SubscriptionPhaseId),
    /// A dated add-on or discount.
    Item(uuid::Uuid),
    /// The term's renewal.
    Renewal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteEntry {
    pub owner: TimelineOwner,
    pub label: String,
    pub on: NaiveDate,
    pub effect: RewriteEffect,
}

/// What a change does to a subscription's timeline: each owner rewritten, and what the
/// subscription bills ahead without and with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEffect {
    pub entries: Vec<RewriteEntry>,
    pub before: crate::services::timeline::Projection,
    pub after: crate::services::timeline::Projection,
}

/// `price` scaled by `factor`, in minor units of a currency of `precision` decimals; one priced
/// finer than that (a sub-cent rate) keeps the catalog's 8 decimals.
pub fn scaled_price(
    price: rust_decimal::Decimal,
    factor: rust_decimal::Decimal,
    precision: u32,
) -> rust_decimal::Decimal {
    let decimals = if price.normalize().scale() > precision {
        8
    } else {
        precision
    };
    (price * factor).round_dp(decimals)
}

/// The decimals of `currency`'s minor unit; 2 for one not known.
pub fn precision_of(currency: &str) -> u32 {
    u32::from(crate::constants::Currencies::resolve_currency_precision(currency).unwrap_or(2))
}

#[cfg(test)]
mod scaled_price_tests {
    use super::scaled_price;
    use rust_decimal_macros::dec;

    #[test]
    fn a_scaled_price_is_in_the_currency_s_minor_units_unless_priced_finer() {
        assert_eq!(scaled_price(dec!(10.70), dec!(1.07), 2), dec!(11.45));
        assert_eq!(scaled_price(dec!(1000), dec!(1.07), 0), dec!(1070));
        assert_eq!(scaled_price(dec!(0.0004), dec!(1.05), 2), dec!(0.00042));
    }
}

/// A quote's contract term (ADR-0022 §2): written as its subscription's term on conversion, its
/// ends counted in months from the billing start.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QuoteTerm {
    pub term_months: u32,
    pub auto_renew: bool,
    pub renewal_months: u32,
    pub notice_days: u32,
    pub uplift_percent: Option<rust_decimal::Decimal>,
}

impl QuoteTerm {
    pub fn from_row(row: &diesel_models::trial_offers::QuoteTermRow) -> Self {
        let months = |m: i32| u32::try_from(m).unwrap_or_default();
        QuoteTerm {
            term_months: months(row.term_months),
            auto_renew: row.auto_renew,
            renewal_months: months(row.renewal_months),
            notice_days: months(row.notice_days),
            uplift_percent: row.uplift_percent,
        }
    }
}
