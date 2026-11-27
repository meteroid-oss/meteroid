//! A subscription renews on the shortest cadence of the fees it bills, as at creation. A fee added
//! or removed later moves it onto that cadence's grid from its billing start, as if it had been
//! created with that set of fees: a longer fee keeps billing on its own anniversaries.

use chrono::NaiveDate;

use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{Period, SubscriptionDetails};
use crate::services::invoice_lines::cadence::{CadenceSubject, cadence_runs_of};
use crate::store::PgConn;
use crate::utils::periods::{calculate_elapsed_cycles, find_period_containing_date};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cadence {
    pub period: BillingPeriodEnum,
    pub current: Period,
    pub cycle_index: u32,
}

/// The cadence fees of `cadences` renew on at `on`, when it is not `period`, with the period
/// holding `on` on its grid.
pub(crate) fn realigned_cadence<'a>(
    period: &BillingPeriodEnum,
    billing_start: NaiveDate,
    billing_day: u32,
    cadences: impl IntoIterator<Item = &'a SubscriptionFeeBillingPeriod>,
    on: NaiveDate,
) -> Option<Cadence> {
    let shortest = cadences
        .into_iter()
        .filter_map(SubscriptionFeeBillingPeriod::as_billing_period_opt)
        .min_by_key(BillingPeriodEnum::as_months)?;
    if shortest.as_months() == period.as_months() {
        return None;
    }
    Some(Cadence {
        current: find_period_containing_date(billing_start, on, &shortest, billing_day),
        cycle_index: calculate_elapsed_cycles(billing_start, on, &shortest, billing_day),
        period: shortest,
    })
}

/// The cadence an active subscription renews on from `on` once it bills fees of `cadences`, unless
/// a plan switch moved it onto a grid of its own.
pub(crate) async fn cadence_after_change(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    cadences: &[SubscriptionFeeBillingPeriod],
    on: NaiveDate,
) -> StoreResult<Option<Cadence>> {
    use crate::domain::SubscriptionStatusEnum;
    let sub = &details.subscription;
    // A change dated on or after the trial's end finds the subscription active.
    let trial_over_by = sub.status == SubscriptionStatusEnum::TrialActive
        && details
            .trial_config
            .as_ref()
            .and_then(|trial| trial.ends_on)
            .is_some_and(|ends_on| ends_on <= on);
    if (sub.status != SubscriptionStatusEnum::Active && !trial_over_by)
        || sub.current_period_end.is_none()
    {
        return Ok(None);
    }
    let Some(cadence) = realigned_cadence(
        &sub.period,
        sub.billing_start_date.unwrap_or(sub.current_period_start),
        u32::from(sub.billing_day_anchor),
        cadences,
        on,
    ) else {
        return Ok(None);
    };
    let switched = switched_onto_own_grid(conn, CadenceSubject::of(details), on).await?;
    Ok((!switched).then_some(cadence))
}

/// Whether a plan switch by `on` moved the subscription onto a grid of its own. A switch on its
/// billing start, one ending a free trial, starts the billing on that start's grid.
pub(crate) async fn switched_onto_own_grid(
    conn: &mut PgConn,
    subject: CadenceSubject,
    on: NaiveDate,
) -> StoreResult<bool> {
    let runs = cadence_runs_of(conn, subject, on, true).await?;
    if runs.is_empty() {
        return Ok(false);
    }
    let trial_end = diesel_models::subscription_facts::SubscriptionTrialRow::find(
        conn,
        subject.subscription_id,
    )
    .await
    .map_err(Into::<error_stack::Report<crate::errors::StoreError>>::into)?
    .filter(|trial| trial.free)
    .map(|trial| trial.until_date);
    let billing_from = match (subject.billing_start, trial_end) {
        (Some(start), Some(end)) => Some(start.max(end)),
        (start, end) => start.or(end),
    };
    Ok(runs
        .iter()
        .any(|run| billing_from.is_none_or(|start| run.switched_on > start)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_monthly_fee_moves_a_yearly_subscription_onto_the_month_holding_the_change() {
        let cadence = realigned_cadence(
            &BillingPeriodEnum::Annual,
            day(2026, 5, 1),
            1,
            &[
                SubscriptionFeeBillingPeriod::Annual,
                SubscriptionFeeBillingPeriod::Monthly,
            ],
            day(2026, 10, 22),
        );
        assert_eq!(
            cadence,
            Some(Cadence {
                period: BillingPeriodEnum::Monthly,
                current: Period {
                    start: day(2026, 10, 1),
                    end: day(2026, 11, 1)
                },
                cycle_index: 5,
            })
        );
    }

    #[test]
    fn removing_the_last_monthly_fee_moves_it_back_onto_the_year() {
        let cadence = realigned_cadence(
            &BillingPeriodEnum::Monthly,
            day(2026, 5, 1),
            1,
            &[SubscriptionFeeBillingPeriod::Annual],
            day(2027, 2, 16),
        );
        assert_eq!(
            cadence,
            Some(Cadence {
                period: BillingPeriodEnum::Annual,
                current: Period {
                    start: day(2026, 5, 1),
                    end: day(2027, 5, 1)
                },
                cycle_index: 0,
            })
        );
    }

    #[test]
    fn the_same_cadence_or_no_recurring_fee_changes_nothing() {
        let monthly = [SubscriptionFeeBillingPeriod::Monthly];
        let one_time = [SubscriptionFeeBillingPeriod::OneTime];
        for cadences in [&monthly[..], &one_time[..]] {
            assert_eq!(
                realigned_cadence(
                    &BillingPeriodEnum::Monthly,
                    day(2026, 5, 1),
                    1,
                    cadences,
                    day(2026, 10, 22),
                ),
                None
            );
        }
    }
}
