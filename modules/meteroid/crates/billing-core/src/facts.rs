//! A subscription as dated facts: rows valid over `[from, to)` or at an instant, written when
//! decided and never changed by the lifecycle. What it bills is `evaluate`'s, from these alone.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use common_domain::ids::{
    AppliedCouponId, PlanVersionId, ProductId, SlotTransactionId, SubscriptionId,
};
use uuid::Uuid;

use crate::charge::Fee;

/// Everything a subscription bills from, generic over the store's fee `F`, which only the
/// [`Pricer`](crate::pricer::Pricer) reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts<F> {
    pub subscription: SubscriptionId,
    pub billing_day_anchor: u32,
    pub start: Start,
    pub trial: Option<Trial>,
    pub plan_windows: Vec<PlanWindow>,
    pub fees: Vec<FeeFact<F>>,
    pub seats: Vec<SeatStep>,
    pub coupons: Vec<CouponFact>,
    pub minimums: Vec<MinimumFact>,
    pub ending: Option<Ending>,
    /// A fixed term's last day: the subscription completes there unless an ending comes first.
    pub end_date: Option<NaiveDate>,
    /// The reconcile watermark, not a fact: a period ending on or before it is never computed
    /// again, one straddling it is computed whole so reconcile sees its whole key.
    pub reconcile_from: Option<NaiveDate>,
    /// Earlier than the watermark for what is billed at a period's end (fixed arrears, usage): an
    /// import's period, its advance billed elsewhere, still owes what it consumed.
    pub arrears_from: Option<NaiveDate>,
    /// The day the cancellation was requested: requested after its date, it is backdated, and what
    /// it ends is billed on a final invoice issued then, never on a document dated in the past.
    pub ending_recorded: Option<NaiveDate>,
    /// The recurring invoice of the cancellation's day takes no more fee lines, what the ending cuts
    /// going on a final invoice: written before the cancellation was requested, or, for a backdated
    /// one, kept (a draft it closes hands its charges to the final invoice).
    pub ending_day_invoiced: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Start {
    /// The billing start when no free trial sets one, however late the start settles: a checkout
    /// bills the period this date opens (R-084).
    pub date: NaiveDate,
    /// When the start was settled: at insert for an immediate start, else by the checkout or the
    /// payment that settled it. Until then only the opening invoice is billed.
    pub settled: Option<DateTime<Utc>>,
    /// A checkout session of the subscription not completed: unsettled, it awaits that checkout.
    pub checkout: bool,
}

/// `[from, until)`; a free trial bills nothing, its freeness read from the plan version in
/// force at the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trial {
    pub from: NaiveDate,
    pub until: NaiveDate,
    pub free: bool,
}

/// A plan version in force over `[from, to)`, its renewals every `cadence` months on a grid
/// from `cadence_origin` (R-084).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanWindow {
    pub plan_version: PlanVersionId,
    pub from: NaiveDate,
    pub to: Option<NaiveDate>,
    pub cadence: u32,
    pub cadence_origin: NaiveDate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Billing {
    Advance,
    Arrears,
    OneTime,
}

/// A fee row: a component or add-on of lineage `lineage` in force over `[from, to)`. Of two
/// rows of one lineage in force on a day, the later one bills.
#[derive(Clone, Debug, PartialEq)]
pub struct FeeFact<F> {
    pub id: Uuid,
    pub lineage: Fee,
    pub from: NaiveDate,
    pub to: Option<NaiveDate>,
    pub billing: Billing,
    /// Months of the fee's own grid (R-084); a one-time fee's is ignored.
    pub cadence: u32,
    /// A component of the plan version it was written with, not one an amendment added.
    pub plan_component: bool,
    pub fee: F,
}

/// From `at` on, the fee of `lineage` bills `quantity` seats, until a later step. A change made
/// during a day counts that whole day, as its purchase bills from its day (OQ24); a renewal
/// written at that day's midnight bills the seats before it, the change billed apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatStep {
    pub id: SlotTransactionId,
    pub lineage: Fee,
    pub at: NaiveDate,
    pub quantity: u64,
    /// Of the seats `quantity` no longer counts, those a removal scheduled for a period's end
    /// takes off: they stay billed until the first renewal on or after `at` (R-108).
    pub removed_at_renewal: u64,
}

/// From `from` on, for `invoices` invoices (forever when none); the count used is read from the
/// issued documents, never kept here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CouponFact {
    pub id: AppliedCouponId,
    pub from: NaiveDate,
    pub invoices: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MinimumScope {
    AllComponents,
    Product(ProductId),
}

impl MinimumScope {
    /// The fee its commitment and shortfall are charged under.
    pub fn fee(&self) -> Fee {
        match self {
            MinimumScope::AllComponents => Fee::Minimum { product: None },
            MinimumScope::Product(id) => Fee::Minimum { product: Some(*id) },
        }
    }
}

/// A minimum commitment of `amount` a period over `[from, to)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MinimumFact {
    pub id: Uuid,
    pub from: NaiveDate,
    pub to: Option<NaiveDate>,
    pub amount: i64,
    pub scope: MinimumScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EndingKind {
    Cancel,
    Complete,
}

/// The subscription ends at `at`: fees on its date, usage at the instant (R-016).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ending {
    pub at: DateTime<Utc>,
    pub kind: EndingKind,
}

impl Ending {
    pub fn on(&self) -> NaiveDate {
        self.at.date_naive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FactKind {
    Start,
    Trial,
    PlanWindow,
    Fee,
    Seat,
    Coupon,
    Minimum,
    Ending,
    EndDate,
}

/// The fact a charge was computed from, for its trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FactRef {
    pub kind: FactKind,
    pub id: Uuid,
}

/// `[from, to)`, open-ended without `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: NaiveDate,
    pub to: Option<NaiveDate>,
}

impl Span {
    pub fn contains(&self, at: NaiveDate) -> bool {
        self.from <= at && self.to.is_none_or(|to| at < to)
    }
}

/// A fact of any kind, as the cutting pass reads it: when it is in force, and the dates it
/// changes what a fee bills on. A new kind of fact is one more impl.
pub trait DatedFact {
    fn reference(&self) -> FactRef;
    fn window(&self) -> Span;
    /// Where a fee's windows are cut into terms: by default where the fact starts and ends.
    fn cuts(&self) -> Vec<NaiveDate> {
        let window = self.window();
        std::iter::once(window.from).chain(window.to).collect()
    }
    /// Whether the fact bears on what the fee of `lineage` bills, so its charges' trace names it.
    fn bears_on(&self, lineage: &Fee) -> bool;
}

impl DatedFact for Start {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::Start,
            id: Uuid::nil(),
        }
    }
    fn window(&self) -> Span {
        Span {
            from: self.date,
            to: None,
        }
    }
    fn bears_on(&self, _: &Fee) -> bool {
        true
    }
}

impl DatedFact for Trial {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::Trial,
            id: Uuid::nil(),
        }
    }
    fn window(&self) -> Span {
        Span {
            from: self.from,
            to: Some(self.until),
        }
    }
    fn bears_on(&self, _: &Fee) -> bool {
        self.free
    }
}

impl DatedFact for PlanWindow {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::PlanWindow,
            id: *self.plan_version,
        }
    }
    fn window(&self) -> Span {
        Span {
            from: self.from,
            to: self.to,
        }
    }
    fn bears_on(&self, _: &Fee) -> bool {
        true
    }
}

impl<F> DatedFact for FeeFact<F> {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::Fee,
            id: self.id,
        }
    }
    /// A one-time fee's row closed on the day it opened still lived that day, which it bills
    /// (R-093): a change the same day never gives it back.
    fn window(&self) -> Span {
        let to = match self.to {
            Some(to) if to == self.from && self.billing == Billing::OneTime => to.succ_opt(),
            to => to,
        };
        Span {
            from: self.from,
            to,
        }
    }
    fn cuts(&self) -> Vec<NaiveDate> {
        std::iter::once(self.from).chain(self.to).collect()
    }
    fn bears_on(&self, lineage: &Fee) -> bool {
        self.lineage == *lineage
    }
}

impl DatedFact for SeatStep {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::Seat,
            id: *self.id,
        }
    }
    fn window(&self) -> Span {
        Span {
            from: self.at,
            to: None,
        }
    }
    fn bears_on(&self, lineage: &Fee) -> bool {
        self.lineage == *lineage
    }
}

impl DatedFact for CouponFact {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::Coupon,
            id: *self.id,
        }
    }
    fn window(&self) -> Span {
        Span {
            from: self.from,
            to: None,
        }
    }
    /// A coupon changes no charge: assembly applies it to the documents.
    fn cuts(&self) -> Vec<NaiveDate> {
        Vec::new()
    }
    fn bears_on(&self, _: &Fee) -> bool {
        false
    }
}

impl DatedFact for MinimumFact {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::Minimum,
            id: self.id,
        }
    }
    fn window(&self) -> Span {
        Span {
            from: self.from,
            to: self.to,
        }
    }
    fn bears_on(&self, lineage: &Fee) -> bool {
        self.scope.fee() == *lineage
    }
}

impl DatedFact for Ending {
    fn reference(&self) -> FactRef {
        FactRef {
            kind: FactKind::Ending,
            id: Uuid::nil(),
        }
    }
    fn window(&self) -> Span {
        Span {
            from: self.on(),
            to: None,
        }
    }
    fn bears_on(&self, _: &Fee) -> bool {
        true
    }
}

pub(crate) fn midnight(date: NaiveDate) -> DateTime<Utc> {
    date.and_time(NaiveTime::MIN).and_utc()
}

impl<F> Facts<F> {
    /// Every fact, as the cutting pass reads it.
    pub fn dated(&self) -> impl Iterator<Item = &dyn DatedFact> {
        let start: [&dyn DatedFact; 1] = [&self.start];
        start
            .into_iter()
            .chain(self.trial.iter().map(|t| t as &dyn DatedFact))
            .chain(self.plan_windows.iter().map(|w| w as &dyn DatedFact))
            .chain(self.fees.iter().map(|f| f as &dyn DatedFact))
            .chain(self.seats.iter().map(|s| s as &dyn DatedFact))
            .chain(self.coupons.iter().map(|c| c as &dyn DatedFact))
            .chain(self.minimums.iter().map(|m| m as &dyn DatedFact))
            .chain(self.ending.iter().map(|e| e as &dyn DatedFact))
    }

    /// The trial when it bills nothing.
    pub fn free_trial(&self) -> Option<&Trial> {
        self.trial.as_ref().filter(|trial| trial.free)
    }

    /// Where fees start billing: the end of the free trial, else the start's date, settled or
    /// not, however late (R-084, R-102).
    pub fn billing_start(&self) -> NaiveDate {
        match self.free_trial() {
            Some(trial) => trial.until.max(self.start.date),
            None => self.start.date,
        }
    }

    /// Whether the start was settled by `now`.
    pub fn settled_by(&self, now: DateTime<Utc>) -> bool {
        self.start.settled.is_some_and(|at| at <= now)
    }

    /// Whether the billing start is a free trial's end, whose first period runs a full cadence on
    /// to the anchor (R-084).
    pub fn bills_from_trial_end(&self) -> bool {
        self.free_trial()
            .is_some_and(|trial| trial.until > self.start.date)
    }

    /// The ending in force: the ending fact or the fixed term's end, the earlier one; a
    /// cancellation dated on the term's end leaves it completed.
    pub fn effective_ending(&self) -> Option<Ending> {
        let term = self.end_date.map(|date| Ending {
            at: midnight(date),
            kind: EndingKind::Complete,
        });
        match (self.ending, term) {
            (Some(ending), Some(term)) => Some(if term.at <= ending.at { term } else { ending }),
            (ending, term) => ending.or(term),
        }
    }

    /// The fact ending the subscription, for a trace.
    pub fn ending_ref(&self) -> Option<FactRef> {
        let ending = self.effective_ending()?;
        Some(match self.ending {
            Some(fact) if fact == ending => fact.reference(),
            _ => FactRef {
                kind: FactKind::EndDate,
                id: Uuid::nil(),
            },
        })
    }

    /// The row of `lineage` billing on `at`: of those in force, the latest to start, the later
    /// written on a tie.
    pub fn fee_on(&self, lineage: &Fee, at: NaiveDate) -> Option<&FeeFact<F>> {
        self.fees
            .iter()
            .filter(|row| row.lineage == *lineage && row.window().contains(at))
            .max_by_key(|row| (row.from, row.id))
    }

    /// The seats of `lineage` on `at`: the latest step on or before it.
    pub fn seats_on(&self, lineage: &Fee, at: NaiveDate) -> Option<&SeatStep> {
        self.seats
            .iter()
            .filter(|step| step.lineage == *lineage && step.at <= at)
            .max_by_key(|step| (step.at, step.id))
    }

    /// The minimum of `scope` on `at`: the latest to start of those in force.
    pub fn minimum_on(&self, scope: MinimumScope, at: NaiveDate) -> Option<&MinimumFact> {
        self.minimums
            .iter()
            .filter(|minimum| minimum.scope == scope && minimum.window().contains(at))
            .max_by_key(|minimum| (minimum.from, minimum.id))
    }

    /// The plan window in force on `at`: the latest to start on or before it, else the first.
    pub fn plan_on(&self, at: NaiveDate) -> Option<&PlanWindow> {
        self.plan_windows
            .iter()
            .filter(|window| window.from <= at)
            .max_by_key(|window| (window.from, window.plan_version))
            .or_else(|| self.plan_windows.iter().min_by_key(|window| window.from))
    }

    /// The coupons in force on `at`, before their invoice counts.
    pub fn coupons_on(&self, at: NaiveDate) -> impl Iterator<Item = &CouponFact> {
        self.coupons.iter().filter(move |coupon| coupon.from <= at)
    }
}
