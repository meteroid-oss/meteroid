//! The timeline's windows (R-084) checked against every period an invoice bills, in tests only
//! (feature `timeline-shadow`): each period not one of its fee's windows fails the test.

use std::sync::Mutex;

use billing_core::timeline::Grid;
use chrono::NaiveDate;

use crate::domain::subscription_components::SubscriptionFeeInterface;
use crate::domain::{ComponentPeriods, Period, SubscriptionDetails};
use crate::utils::periods::CadenceRun;

static MISMATCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What the timeline found different from the invoices since the last call.
pub fn take_mismatches() -> Vec<String> {
    MISMATCHES
        .lock()
        .map(|mut found| std::mem::take(&mut *found))
        .unwrap_or_default()
}

/// The grid a record bills on: the latest change of cadence before it ended, else the billing
/// start's, at its own cadence.
fn grid_of<T: SubscriptionFeeInterface>(
    record: &T,
    runs: &[CadenceRun],
    billing_start: NaiveDate,
    anchor: u32,
) -> Option<Grid> {
    let months = record.period_ref().as_billing_period_opt()?.as_months();
    match runs
        .iter()
        .find(|run| record.effective_to().is_none_or(|to| to > run.switched_on))
    {
        Some(run) => Grid::from_origin(run.origin, anchor, months),
        None => Grid::from_start(billing_start, anchor, months),
    }
}

/// Checks that each period `periods` bills for `record` on the invoice of `invoice_date` is one of
/// the record's windows as its life read when billed: within one grid period, from its start or the
/// record's, to its end or the record's.
pub(crate) fn check<T: SubscriptionFeeInterface>(
    details: &SubscriptionDetails,
    record: &T,
    periods: &ComponentPeriods,
    invoice_date: NaiveDate,
    billing_start: NaiveDate,
    runs: &[CadenceRun],
) {
    let sub = &details.subscription;
    let anchor = u32::from(sub.billing_day_anchor);
    let Some(grid) = grid_of(record, runs, billing_start, anchor) else {
        return;
    };
    let completed = sub.current_period_end.is_none() && !sub.pending_checkout;
    let billed: Vec<(&str, &Period)> = periods
        .advance
        .iter()
        .map(|p| ("advance", p))
        .chain(periods.arrear.iter().map(|p| ("arrears", p)))
        .filter(|(_, p)| p.start < p.end)
        .collect();
    let mut differences = Vec::new();
    let (from, to) = (record.effective_from(), record.effective_to());
    let trial_end = sub.trial_duration.and_then(|days| {
        sub.start_date
            .checked_add_days(chrono::Days::new(u64::from(days)))
    });
    for (side, period) in billed {
        let Some(grid_period) = grid.period_containing(period.start) else {
            continue;
        };
        let starts = period.start == grid_period.0
            || Some(period.start) == from
            || period.start == billing_start
            || Some(period.start) == trial_end;
        let ends = period.end == grid_period.1
            || Some(period.end) == to
            || (completed && side == "arrears" && period.end == invoice_date);
        if !(starts && ends && period.end <= grid_period.1) {
            differences.push(format!(
                "{} {} {side} {}..{} on the invoice of {invoice_date} is no window of its fee: \
                 grid period {:?}, life {from:?}..{to:?}, billing start {billing_start}",
                sub.id,
                record.name_ref(),
                period.start,
                period.end,
                grid_period,
            ));
        }
    }
    for difference in differences {
        log::warn!("timeline shadow: {difference}");
        if let Ok(mut found) = MISMATCHES.lock() {
            found.push(difference);
        }
    }
}
