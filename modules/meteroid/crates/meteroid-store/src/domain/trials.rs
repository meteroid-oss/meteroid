//! Trials (ADR 0015 §3, R-153, R-159): one definition, the trial offer. A plan's trial is the
//! offer it names; a subscription's trial is its fact, fixed at its start.
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{PlanId, PlanVersionId, TrialOfferId};
use diesel_models::trial_offers::TrialOfferRow;
use error_stack::Report;

use crate::errors::{StoreError, StoreErrorReport};

/// The longest trial, in days, from its billing start.
pub const MAX_TRIAL_DAYS: u32 = 3650;

/// The days of notice a trial's end is given (`subscription.trial_ending_soon`, the
/// `TrialEnding` email): at its start when it is shorter (R-158).
pub const TRIAL_NOTICE_DAYS: i32 = 7;

/// A trial is free (R-153): a discounted start is a coupon or an introductory phase.
pub const TRIAL_IS_FREE: &str =
    "a trial is free: a discounted start is a coupon or an introductory phase";

/// A trial stated with its freeness, as a quote or a checkout session states it: never billed.
pub fn refuse_billed(free: Option<bool>) -> Result<(), StoreErrorReport> {
    match free {
        Some(false) => Err(crate::errors::TrialRefusal::Billed.report(TRIAL_IS_FREE)),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialEndAction {
    /// The subscription goes on, billed from the trial's end (`TrialWithoutCard` without a card).
    Convert,
    /// It ends with the trial.
    Cancel,
    /// It switches to another plan at the trial's end.
    Fallback { plan_version_id: PlanVersionId },
    /// Nothing is billed at the trial's end: the trial's access holds until an operator converts,
    /// extends or cancels it.
    Manual,
}

impl TrialEndAction {
    pub fn kind(&self) -> TrialEnd {
        match self {
            TrialEndAction::Convert => TrialEnd::Convert,
            TrialEndAction::Cancel => TrialEnd::Cancel,
            TrialEndAction::Fallback { .. } => TrialEnd::Fallback,
            TrialEndAction::Manual => TrialEnd::Manual,
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
    /// Its end waits for an operator's decision, billing nothing meanwhile.
    Manual,
}

impl TrialEnd {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrialEnd::Convert => "CONVERT",
            TrialEnd::Cancel => "CANCEL",
            TrialEnd::Fallback => "FALLBACK",
            TrialEnd::Manual => "MANUAL",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "CANCEL" => TrialEnd::Cancel,
            "FALLBACK" => TrialEnd::Fallback,
            "MANUAL" => TrialEnd::Manual,
            _ => TrialEnd::Convert,
        }
    }
}

/// What a converting trial's end does when the subscription, charged automatically, resolves no
/// payment method then (R-179).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrialWithoutCard {
    /// TrialExpired, its opening invoice issued: Active once paid.
    #[default]
    AwaitPayment,
    /// It ends on the trial's end, nothing billed.
    Cancel,
    /// It switches to this plan version on the trial's end.
    Switch { plan_version_id: PlanVersionId },
}

impl TrialWithoutCard {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrialWithoutCard::AwaitPayment => "AWAIT_PAYMENT",
            TrialWithoutCard::Cancel => "CANCEL",
            TrialWithoutCard::Switch { .. } => "SWITCH",
        }
    }

    pub fn plan_version_id(&self) -> Option<PlanVersionId> {
        match self {
            TrialWithoutCard::Switch { plan_version_id } => Some(*plan_version_id),
            _ => None,
        }
    }

    pub(crate) fn parse(value: &str, plan_version_id: Option<PlanVersionId>) -> Option<Self> {
        Some(match (value, plan_version_id) {
            ("AWAIT_PAYMENT", _) => TrialWithoutCard::AwaitPayment,
            ("CANCEL", _) => TrialWithoutCard::Cancel,
            ("SWITCH", Some(plan_version_id)) => TrialWithoutCard::Switch { plan_version_id },
            _ => return None,
        })
    }
}

/// How a trial ended, recorded by the billing controller at its end, by an end now, or when an
/// awaiting outcome resolves (R-167).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrialOutcome {
    Converted,
    Cancelled,
    Switched,
    /// No payment method at its end: TrialExpired until its opening invoice is paid.
    AwaitingPayment,
    /// A manual end: the trial's access holds until an operator decides.
    AwaitingDecision,
}

impl TrialOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrialOutcome::Converted => "CONVERTED",
            TrialOutcome::Cancelled => "CANCELLED",
            TrialOutcome::Switched => "SWITCHED",
            TrialOutcome::AwaitingPayment => "AWAITING_PAYMENT",
            TrialOutcome::AwaitingDecision => "AWAITING_DECISION",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "CONVERTED" => TrialOutcome::Converted,
            "CANCELLED" => TrialOutcome::Cancelled,
            "SWITCHED" => TrialOutcome::Switched,
            "AWAITING_PAYMENT" => TrialOutcome::AwaitingPayment,
            "AWAITING_DECISION" => TrialOutcome::AwaitingDecision,
            _ => return None,
        })
    }

    /// Still to resolve: a payment or an operator's decision.
    pub fn is_awaiting(&self) -> bool {
        matches!(
            self,
            TrialOutcome::AwaitingPayment | TrialOutcome::AwaitingDecision
        )
    }
}

/// What a subscription bills from its trial's end: the recurring fees of its opening invoice
/// from then, one amount per cadence, before coupons and tax, usage and one-time fees aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AfterTrial {
    pub currency: String,
    pub starts_on: NaiveDate,
    /// The subscription's own cadence first, then the others shortest first.
    pub recurring: Vec<RecurringAmount>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecurringAmount {
    pub amount: i64,
    pub period: crate::domain::enums::BillingPeriodEnum,
}

/// A subscription's trial as its customer reads it, while it runs or awaits a payment or a
/// decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerTrial {
    /// Its end, exclusive.
    pub ends_on: NaiveDate,
    pub status: TrialStatus,
    /// What follows a running converting trial.
    pub after_trial: Option<AfterTrial>,
    pub has_payment_method: bool,
    /// The customer's payment method it charges ("Visa •••• 4242").
    pub payment_method: Option<String>,
    /// When it ended, awaiting a payment or a decision.
    pub ended_on: Option<NaiveDate>,
    /// Awaiting payment: the opening invoice that converts it once paid.
    pub invoice_to_pay: Option<TrialInvoiceToPay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialInvoiceToPay {
    pub invoice_id: common_domain::ids::InvoiceId,
    pub invoice_number: String,
    pub amount_due: i64,
    pub currency: String,
    pub due_at: Option<NaiveDateTime>,
}

/// The trials a subscription list narrows to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialListFilter {
    /// Running and ending within this many days from today.
    EndingWithinDays(u32),
    AwaitingDecision,
    AwaitingPayment,
}

impl TrialListFilter {
    pub fn row_filter(
        self,
        today: NaiveDate,
    ) -> diesel_models::query::subscriptions::TrialListFilter {
        use diesel_models::query::subscriptions::TrialListFilter as Row;
        match self {
            TrialListFilter::EndingWithinDays(days) => Row::EndingBetween {
                after: today,
                until: today
                    .checked_add_days(chrono::Days::new(u64::from(days)))
                    .unwrap_or(NaiveDate::MAX),
            },
            TrialListFilter::AwaitingDecision => {
                Row::Outcome(TrialOutcome::AwaitingDecision.as_str())
            }
            TrialListFilter::AwaitingPayment => {
                Row::Outcome(TrialOutcome::AwaitingPayment.as_str())
            }
        }
    }
}

/// A listed subscription's trial: when it ends, its status, and while it runs or awaits a
/// payment whether the subscription resolves a payment method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialSummary {
    pub ends_on: NaiveDate,
    pub status: TrialStatus,
    pub has_payment_method: Option<bool>,
    /// The customer's payment method it charges ("Visa •••• 4242"), when it has one.
    pub payment_method: Option<String>,
}

/// A subscription trial's status, read from its outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialStatus {
    Active,
    Converted,
    Cancelled,
    Switched,
    AwaitingPayment,
    AwaitingDecision,
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

    pub(crate) fn parse(value: &str) -> Option<Self> {
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
    /// A default only, for checkout, sign-up and the pricing page: the subscription's activation
    /// condition decides.
    pub card_required: bool,
    pub end_action: TrialEndAction,
    pub without_card: TrialWithoutCard,
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
        let without_card =
            TrialWithoutCard::parse(&row.without_card, row.without_card_plan_version_id)
                .ok_or_else(|| bad("unknown end without a card"))?;
        Ok(TrialOffer {
            id: row.id,
            name: row.name,
            code: row.code,
            // 0: the offer gives no trial (any more): resolution never gives it (`offers`).
            duration_days: row
                .duration_days
                .map_or(0, |days| u32::try_from(days).unwrap_or(1)),
            free: row.free,
            trial_plan_id: row.trial_plan_id,
            card_required: row.card_required,
            end_action,
            without_card,
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
        ("MANUAL", _) => TrialEndAction::Manual,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct TrialOfferNew {
    pub name: String,
    pub code: Option<String>,
    pub duration_days: u32,
    /// Always true: a billed trial is refused (`TRIAL_IS_FREE`).
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
    pub card_required: bool,
    pub end_action: TrialEndAction,
    pub without_card: TrialWithoutCard,
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
    pub card_required: Option<bool>,
    pub end_action: Option<TrialEndAction>,
    pub without_card: Option<TrialWithoutCard>,
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
            card_required: patch.card_required.unwrap_or(self.card_required),
            end_action: patch.end_action.unwrap_or(self.end_action),
            without_card: patch.without_card.unwrap_or(self.without_card),
            eligibility: patch.eligibility.unwrap_or(self.eligibility),
        }
    }

    pub fn terms(&self) -> TrialTerms {
        TrialTerms {
            duration_days: self.duration_days,
            free: self.free,
            trial_plan_id: self.trial_plan_id,
            card_required: Some(self.card_required),
            end_action: Some(self.end_action),
            without_card: Some(self.without_card),
        }
    }
}

/// What a plan's trial gives, as the plan editor writes it; the rest of its offer is kept, and so
/// is each of the last three left `None` (else card required, convert, await payment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialTerms {
    pub duration_days: u32,
    /// Always true: a billed trial is refused (`TRIAL_IS_FREE`).
    pub free: bool,
    pub trial_plan_id: Option<PlanId>,
    pub card_required: Option<bool>,
    pub end_action: Option<TrialEndAction>,
    pub without_card: Option<TrialWithoutCard>,
}

impl TrialTerms {
    /// Free terms of `duration_days`, the rest kept.
    pub fn days(duration_days: u32, trial_plan_id: Option<PlanId>) -> Self {
        TrialTerms {
            duration_days,
            free: true,
            trial_plan_id,
            card_required: None,
            end_action: None,
            without_card: None,
        }
    }

    /// Every term set: those left `None` taken from `current`, else the defaults.
    pub fn over(self, current: Option<&TrialTerms>) -> TrialTerms {
        TrialTerms {
            card_required: self
                .card_required
                .or(current.and_then(|c| c.card_required))
                .or(Some(true)),
            end_action: self
                .end_action
                .or(current.and_then(|c| c.end_action))
                .or(Some(TrialEndAction::Convert)),
            without_card: self
                .without_card
                .or(current.and_then(|c| c.without_card))
                .or(Some(TrialWithoutCard::AwaitPayment)),
            ..self
        }
    }
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

/// The trial a creation asks for (R-153). The plan's trial asked by default gives none to a
/// customer who already had it; asked explicitly (`Plan`) or named (`Offer`), it is refused (R-159).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TrialRequest {
    /// Nothing asked: the plan's trial, when the customer may still have it.
    #[default]
    Default,
    /// The plan's trial, asked explicitly.
    Plan,
    None,
    Offer(TrialOfferId),
    /// Its own terms; freeness and trial plan the plan's trial's when left out, else free, none.
    Custom {
        length: TrialLength,
        free: Option<bool>,
        trial_plan: Option<Option<PlanId>>,
        /// Its end waits for an operator's decision ("decide at the end"), else it converts.
        manual: bool,
    },
}

impl TrialRequest {
    /// A request in days as the older APIs give it: none asks nothing, 0 none.
    pub fn from_days(days: Option<u32>, free: Option<bool>) -> Self {
        match days {
            None => TrialRequest::Default,
            Some(0) => TrialRequest::None,
            Some(days) => TrialRequest::Custom {
                length: TrialLength::Days(days),
                free,
                trial_plan: None,
                manual: false,
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
    pub without_card: TrialWithoutCard,
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
    /// Today, its recorded end action with it: a converting or manual trial bills from today, a
    /// cancelling one ends the subscription today, a falling back one switches today.
    Now,
    /// The subscription ends today with its trial, nothing billed: a manual trial's cancel.
    Cancel,
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
    /// Applies when `end_action` is Convert (R-179).
    pub without_card: TrialWithoutCard,
    /// Set once it ended (R-167).
    pub outcome: Option<TrialOutcome>,
    pub ended_at: Option<NaiveDateTime>,
}

impl SubscriptionTrial {
    pub fn days(&self) -> u32 {
        u32::try_from((self.until - self.from).num_days()).unwrap_or(0)
    }

    pub fn status(&self) -> TrialStatus {
        match self.outcome {
            None => TrialStatus::Active,
            Some(TrialOutcome::Converted) => TrialStatus::Converted,
            Some(TrialOutcome::Cancelled) => TrialStatus::Cancelled,
            Some(TrialOutcome::Switched) => TrialStatus::Switched,
            Some(TrialOutcome::AwaitingPayment) => TrialStatus::AwaitingPayment,
            Some(TrialOutcome::AwaitingDecision) => TrialStatus::AwaitingDecision,
        }
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
            without_card: TrialWithoutCard::parse(
                &row.without_card,
                row.without_card_plan_version_id,
            )
            .unwrap_or_default(),
            outcome: row.outcome.as_deref().and_then(TrialOutcome::parse),
            ended_at: row.ended_at,
        }
    }
}
