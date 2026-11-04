//! A subscription renews on the shortest cadence of the fees it bills, as at creation. A fee added
//! or removed later moves it onto that cadence's grid from its billing start, as if it had been
//! created with that set of fees: a longer fee keeps billing on its own anniversaries.

use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::{SubscriptionCadenceRowPatch, SubscriptionRow};
use error_stack::Report;

use crate::StoreResult;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use crate::domain::{Period, SubscriptionDetails};
use crate::errors::StoreError;
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
    let sub = &details.subscription;
    if sub.status != crate::domain::SubscriptionStatusEnum::Active
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
    let switched = !cadence_runs_of(conn, CadenceSubject::of(details), on, true)
        .await?
        .is_empty();
    Ok((!switched).then_some(cadence))
}

/// Moves the subscription onto the shortest cadence of the fees it bills from `on`.
pub(crate) async fn realign_cadence_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
) -> StoreResult<Option<Cadence>> {
    let subscription = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .subscription;
    if subscription.status != SubscriptionStatusEnum::Active
        || subscription.current_period_end.is_none()
    {
        return Ok(None);
    }
    let components = SubscriptionComponentRow::list_subscription_components_by_subscription(
        conn,
        &tenant_id,
        &subscription_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    let add_ons =
        SubscriptionAddOnRow::list_by_subscription_id_active(conn, &tenant_id, &subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    let cadences: Vec<SubscriptionFeeBillingPeriod> = components
        .into_iter()
        .map(|c| c.period.into())
        .chain(add_ons.into_iter().map(|a| a.period.into()))
        .collect();
    let Some(cadence) = realigned_cadence(
        &subscription.period.clone().into(),
        subscription
            .billing_start_date
            .unwrap_or(subscription.current_period_start),
        subscription.billing_day_anchor as u32,
        &cadences,
        on,
    ) else {
        return Ok(None);
    };
    if !cadence_runs_of(conn, CadenceSubject::of_row(&subscription), on, true)
        .await?
        .is_empty()
    {
        return Ok(None);
    }

    let (end, next_cycle_action) = match subscription.end_date {
        Some(end_date) if end_date <= cadence.current.end => {
            (end_date, CycleActionEnum::EndSubscription)
        }
        _ => (cadence.current.end, CycleActionEnum::RenewSubscription),
    };
    SubscriptionCadenceRowPatch {
        id: subscription_id,
        tenant_id,
        period: cadence.period.into(),
        current_period_start: cadence.current.start,
        current_period_end: Some(end),
        cycle_index: Some(cadence.cycle_index as i32),
        next_cycle_action: Some(next_cycle_action),
    }
    .patch(conn)
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    log::info!(
        "Subscription {subscription_id} now renews {:?}: period [{}, {end})",
        cadence.period,
        cadence.current.start,
    );
    Ok(Some(cadence))
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
