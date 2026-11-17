//! The controller's step: what applying an evaluation at `now` changes, from the state the last
//! step stored. The state changes it passed through, the state to store, the documents now due and
//! when to look again; the store writes them in one transaction under the customer's lock.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use common_domain::ids::{BillableMetricId, PlanVersionId};

use crate::evaluate::{Charge, DerivedState, DocumentSlot, Evaluation, Status, evaluate};
use crate::facts::{Facts, midnight};
use crate::pricer::{Pricer, Rate, Segment, UsageReads, UsageWindow};
use crate::pricing::PricingError;

/// The derived state's stored columns, as the last step wrote them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stored {
    pub status: Status,
    pub current_period: Option<(NaiveDate, Option<NaiveDate>)>,
    pub plan_version: Option<PlanVersionId>,
    pub cadence: Option<u32>,
    pub pending_checkout: bool,
}

impl From<&DerivedState> for Stored {
    fn from(state: &DerivedState) -> Self {
        Stored {
            status: state.status,
            current_period: state.current_period,
            plan_version: state.plan_version,
            cadence: state.cadence,
            pending_checkout: state.pending_checkout,
        }
    }
}

/// A change of the derived state, at the instant a fact made it: emitted once, keyed by
/// subscription, change and instant, it writes the logs and the outbox event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Status {
        from: Status,
        to: Status,
        at: DateTime<Utc>,
    },
    /// A plan window of another plan version in force from `at`.
    Plan {
        from: PlanVersionId,
        to: PlanVersionId,
        at: DateTime<Utc>,
    },
}

impl Transition {
    pub fn at(&self) -> DateTime<Utc> {
        match self {
            Transition::Status { at, .. } | Transition::Plan { at, .. } => *at,
        }
    }
}

/// A document due by `now` that the subscription has none of yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub slot: DocumentSlot,
    /// A recurring invoice at its period's start, a final one at the ending, an adjustment at its
    /// first charge.
    pub at: DateTime<Utc>,
    pub charges: Vec<Charge>,
    /// The opening invoice of a start still to settle: the checkout's, which the controller
    /// writes only when it can collect it unattended.
    pub awaiting_checkout: bool,
}

/// Deadlines the store reads with the snapshot: those of the subscription's open invoices, and
/// the lapses of credit its usage may draw, still to settle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadlines<'a> {
    pub finalize_after: &'a [DateTime<Utc>],
    pub next_payment_attempt_at: &'a [DateTime<Utc>],
    pub credit_lapses: &'a [DateTime<Utc>],
    /// The invoicing entity's settlement grace (OQ15).
    pub settlement_grace: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// In the order they happened; a controller that ran late emits each one it missed.
    pub transitions: Vec<Transition>,
    /// The state to store, when it differs from the stored one.
    pub state: Option<Stored>,
    pub documents: Vec<Due>,
    /// The earliest of the evaluation's next wake, a document not due yet and the deadlines; a
    /// deadline already passed is due at once.
    pub next_wake_at: Option<DateTime<Utc>>,
}

impl Step {
    /// Nothing to write: a step run again at the same instant is empty.
    pub fn is_empty(&self) -> bool {
        self.transitions.is_empty() && self.state.is_none() && self.documents.is_empty()
    }
}

/// What the controller does at `now`, from `evaluation` (the facts evaluated at `now`), the state
/// the last step stored and the documents written so far, one per slot (R-051, R-052): a written
/// one is refreshed while a draft, else corrected by reconcile, never written again.
///
/// Rules: R-084, R-011, R-013, R-016, OQ9, OQ15
pub fn step<F>(
    facts: &Facts<F>,
    now: DateTime<Utc>,
    evaluation: &Evaluation,
    stored: &Stored,
    written: &[DocumentSlot],
    deadlines: &Deadlines<'_>,
) -> Step {
    let derived = Stored::from(&evaluation.state);
    let mut transitions = status_path(facts, now, stored.status, derived.status);
    transitions.extend(plan_path(
        facts,
        now,
        stored.plan_version,
        derived.plan_version,
    ));
    // Stable: on one instant a status change comes before a plan change.
    transitions.sort_by_key(Transition::at);

    let (documents, opening) = due_documents(now, evaluation, written);
    let next_wake_at = evaluation
        .next_wake
        .into_iter()
        .chain(opening)
        .chain(deadlines.finalize_after.iter().copied())
        .chain(deadlines.next_payment_attempt_at.iter().copied())
        .chain(
            deadlines
                .credit_lapses
                .iter()
                .map(|lapse| *lapse + deadlines.settlement_grace),
        )
        .min();

    Step {
        transitions,
        state: (derived != *stored).then_some(derived),
        documents,
        next_wake_at,
    }
}

/// Prices nothing: the state at an instant needs no price.
struct Unpriced;

impl<F> Pricer<F> for Unpriced {
    fn metrics(&self, _: &F) -> Vec<BillableMetricId> {
        Vec::new()
    }

    fn rates(&self, _: &Segment<'_, F>) -> Result<Vec<Rate>, PricingError> {
        Ok(Vec::new())
    }

    fn usage(
        &self,
        _: &Segment<'_, F>,
        _: &UsageWindow,
        _: &UsageReads,
    ) -> Result<Vec<Rate>, PricingError> {
        Ok(Vec::new())
    }
}

fn status_at<F>(facts: &Facts<F>, at: DateTime<Utc>) -> Option<Status> {
    evaluate(facts, at, None, &Unpriced)
        .ok()
        .map(|evaluation| evaluation.state.status)
}

/// Each status since the stored one, at the instant it began. The status changes only where a
/// fact puts an instant: the start, its settlement, the trial's bounds, the ending.
fn status_path<F>(
    facts: &Facts<F>,
    now: DateTime<Utc>,
    stored: Status,
    derived: Status,
) -> Vec<Transition> {
    if stored == derived {
        return Vec::new();
    }
    let mut instants: Vec<DateTime<Utc>> = std::iter::once(midnight(facts.start.date))
        .chain(facts.start.settled)
        .chain(
            facts
                .trial
                .iter()
                .flat_map(|trial| [midnight(trial.from), midnight(trial.until)]),
        )
        .chain(facts.effective_ending().map(|ending| ending.at))
        .filter(|at| *at <= now)
        .collect();
    instants.sort();
    instants.dedup();

    let mut began: Vec<(DateTime<Utc>, Status)> = Vec::new();
    if let Some(first) = instants.first().copied() {
        let before = first
            .checked_sub_signed(Duration::seconds(1))
            .unwrap_or(first);
        let statuses = std::iter::once((before, status_at(facts, before)))
            .chain(instants.iter().map(|at| (*at, status_at(facts, *at))));
        for (at, status) in statuses {
            if let Some(status) = status {
                push_change(&mut began, at, status);
            }
        }
    }
    push_change(&mut began, now, derived);

    match began.iter().rposition(|(_, status)| *status == stored) {
        Some(i) => began[i..]
            .windows(2)
            .map(|pair| Transition::Status {
                from: pair[0].1,
                to: pair[1].1,
                at: pair[1].0,
            })
            .collect(),
        // A stored status the facts never gave (a legacy write): one change, when the derived
        // status began.
        None => vec![Transition::Status {
            from: stored,
            to: derived,
            at: began
                .last()
                .filter(|_| began.len() > 1)
                .map_or(now, |(at, _)| *at),
        }],
    }
}

fn push_change<T: PartialEq>(began: &mut Vec<(DateTime<Utc>, T)>, at: DateTime<Utc>, value: T) {
    if began.last().is_none_or(|(_, last)| *last != value) {
        began.push((at, value));
    }
}

/// Each plan version since the stored one, from the day its window opened: windows of one
/// version in a row (a change of cadence) are one.
fn plan_path<F>(
    facts: &Facts<F>,
    now: DateTime<Utc>,
    stored: Option<PlanVersionId>,
    derived: Option<PlanVersionId>,
) -> Vec<Transition> {
    let (Some(stored), Some(derived)) = (stored, derived) else {
        return Vec::new();
    };
    if stored == derived {
        return Vec::new();
    }
    let today = now.date_naive();
    let mut windows: Vec<_> = facts
        .plan_windows
        .iter()
        .filter(|window| window.from <= today)
        .collect();
    windows.sort_by_key(|window| (window.from, window.plan_version));
    let mut began: Vec<(DateTime<Utc>, PlanVersionId)> = Vec::new();
    for window in windows {
        push_change(&mut began, midnight(window.from), window.plan_version);
    }
    match began.iter().rposition(|(_, version)| *version == stored) {
        Some(i) => began[i..]
            .windows(2)
            .map(|pair| Transition::Plan {
                from: pair[0].1,
                to: pair[1].1,
                at: pair[1].0,
            })
            .collect(),
        None => vec![Transition::Plan {
            from: stored,
            to: derived,
            at: began.last().map_or(now, |(at, _)| *at),
        }],
    }
}

/// The evaluation's charges by document, those due by `now` and not written; and when the first
/// of the others opens. Until its start settles a subscription has its opening invoice alone, the
/// checkout's (R-011); a cancellation before that has none (R-013).
fn due_documents(
    now: DateTime<Utc>,
    evaluation: &Evaluation,
    written: &[DocumentSlot],
) -> (Vec<Due>, Option<DateTime<Utc>>) {
    let mut slots: BTreeMap<DocumentSlot, Vec<Charge>> = BTreeMap::new();
    for charge in &evaluation.charges {
        slots
            .entry(charge.document)
            .or_default()
            .push(charge.clone());
    }
    let mut due: Vec<Due> = Vec::new();
    let mut opening: Option<DateTime<Utc>> = None;
    for (slot, charges) in slots {
        if written.contains(&slot) {
            continue;
        }
        let first = charges
            .iter()
            .map(|charge| charge.due_at)
            .min()
            .unwrap_or(now);
        // A charge riding the next renewal (R-084) waits for its period; the ending's go on
        // their document when it applies (R-016).
        let at = match slot {
            DocumentSlot::Recurring { period_start } => midnight(period_start),
            DocumentSlot::Final | DocumentSlot::Adjustment { .. } => first,
        };
        if at <= now {
            due.push(Due {
                slot,
                at,
                charges,
                awaiting_checkout: evaluation.state.pending_checkout,
            });
        } else {
            opening = Some(opening.map_or(at, |earliest| earliest.min(at)));
        }
    }
    due.sort_by_key(|document| document.at);
    (due, opening)
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;
