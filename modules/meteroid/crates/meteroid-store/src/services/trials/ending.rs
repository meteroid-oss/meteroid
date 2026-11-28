//! A trial's end decided once it is reached (R-179, R-167): its outcome recorded as a fact the
//! billing core reads, with what a converting end without a payment method writes.
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanVersionId, SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::subscription_facts::{
    SubscriptionEndingRow, SubscriptionPlanWindowRow, SubscriptionTrialRow,
};
use diesel_models::subscriptions::SubscriptionRow;
use scoped_futures::ScopedFutureExt;

use super::db;
use crate::StoreResult;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::subscription_phases::{PhasePlan, PhaseTerms, SubscriptionPhaseInput};
use crate::domain::trials::{TrialEnd, TrialOutcome, TrialWithoutCard};
use crate::domain::usage_alert_events::TrialEndedEvent;
use crate::services::Services;

fn midnight(day: NaiveDate) -> NaiveDateTime {
    day.and_time(NaiveTime::MIN)
}

/// The plan switch a trial's end writes: a phase on `on`, its first renewal (R-150).
pub(in crate::services) async fn write_switch_phase_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    plan_version_id: PlanVersionId,
) -> StoreResult<()> {
    let seq =
        crate::services::subscriptions::phases::list_phases_tx(conn, tenant_id, subscription_id)
            .await?
            .len();
    crate::services::subscriptions::phases::write_phase_tx(
        services,
        conn,
        actor,
        tenant_id,
        subscription_id,
        u32::try_from(seq).unwrap_or(u32::MAX),
        &SubscriptionPhaseInput {
            start_date: on,
            label: Some("After the trial".to_string()),
            terms: PhaseTerms {
                plan: Some(PhasePlan {
                    plan_version_id,
                    option: None,
                    component_params: vec![],
                    name: None,
                }),
                ..PhaseTerms::default()
            },
        },
    )
    .await
    .map(|_| ())
}

/// Records the outcome of the subscription's trial once its end or an ending is reached by `now`,
/// in the caller's transaction under the customer's lock; run again, it writes nothing. An ending
/// on or before the trial's end, or while it awaits a payment or a decision, cancels it.
pub(in crate::services) async fn decide_trial_end_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    row: &SubscriptionRow,
    now: DateTime<Utc>,
) -> StoreResult<Option<TrialOutcome>> {
    let Some(trial) = db(SubscriptionTrialRow::find(conn, row.id).await)? else {
        return Ok(None);
    };
    let recorded = trial.outcome.as_deref().and_then(TrialOutcome::parse);
    if recorded.is_some_and(|outcome| !outcome.is_awaiting()) {
        return Ok(None);
    }
    let now = now.naive_utc();
    let until = midnight(trial.until_date);
    let ended_by = db(SubscriptionEndingRow::list_of(conn, row.id).await)?
        .iter()
        .map(|ending| ending.at)
        .min()
        .filter(|at| *at <= now);
    let decided = match recorded {
        Some(outcome) if outcome.is_awaiting() => {
            ended_by.map(|at| (TrialOutcome::Cancelled, at.max(until)))
        }
        Some(_) => None,
        None => match ended_by.filter(|at| *at <= until) {
            Some(at) => Some((TrialOutcome::Cancelled, at)),
            None if until <= now => {
                Some((at_end_tx(services, conn, actor, row, &trial).await?, until))
            }
            None => None,
        },
    };
    let Some((outcome, at)) = decided else {
        return Ok(None);
    };
    record_tx(conn, row, &trial, outcome, at).await?;
    Ok(Some(outcome))
}

/// A trial awaiting its opening invoice's payment converts once its start settles (R-179): the
/// payment, or a checkout, activating it.
pub(in crate::services) async fn convert_awaiting_payment_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    at: NaiveDateTime,
) -> StoreResult<()> {
    let Some(trial) = db(SubscriptionTrialRow::find(conn, subscription_id).await)? else {
        return Ok(());
    };
    if trial.outcome.as_deref().and_then(TrialOutcome::parse) != Some(TrialOutcome::AwaitingPayment)
    {
        return Ok(());
    }
    let row = db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
        .subscription;
    record_tx(conn, &row, &trial, TrialOutcome::Converted, at).await
}

/// The trial's outcome recorded at `at`, with its `subscription.trial_ended`.
pub(in crate::services) async fn record_tx(
    conn: &mut PgConn,
    row: &SubscriptionRow,
    trial: &SubscriptionTrialRow,
    outcome: TrialOutcome,
    at: NaiveDateTime,
) -> StoreResult<()> {
    crate::services::facts::record_trial_outcome_tx(conn, row.id, Some((outcome, at))).await?;
    crate::repositories::outbox::publish_outbox_events_tx(
        conn,
        vec![OutboxEvent::SubscriptionTrialEnded(Box::new(
            TrialEndedEvent {
                id: TrialEndedEvent::event_id(row.id, trial.until_date, outcome),
                tenant_id: row.tenant_id,
                customer_id: row.customer_id,
                subscription_id: row.id,
                trial_end: trial.until_date,
                outcome,
                ended_at: at,
                trial_offer_id: trial.trial_offer_id.map(|id| id.as_uuid()),
            },
        ))],
    )
    .await?;
    crate::services::customer_emails::trials::enqueue_trial_ended_email_tx(
        conn,
        row.tenant_id,
        row.customer_id,
        row.id,
        at.date(),
        outcome,
    )
    .await
}

/// What the trial's recorded end makes of its end. A converting free trial on a plan billing
/// something, charged automatically with no payment method, takes its `without_card`; a write it
/// cannot make (the switch, the end date) leaves it awaiting payment.
async fn at_end_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    row: &SubscriptionRow,
    trial: &SubscriptionTrialRow,
) -> StoreResult<TrialOutcome> {
    let until = trial.until_date;
    match TrialEnd::parse(&trial.end_action) {
        TrialEnd::Manual => return Ok(TrialOutcome::AwaitingDecision),
        // Its end date moved off the trial's end: it went on.
        TrialEnd::Cancel => return Ok(TrialOutcome::Converted),
        TrialEnd::Fallback => {
            let switches =
                crate::services::subscriptions::phases::list_phases_tx(conn, row.tenant_id, row.id)
                    .await?
                    .iter()
                    .any(|phase| phase.start_date == until && phase.terms.plan.is_some());
            return Ok(if switches {
                TrialOutcome::Switched
            } else {
                TrialOutcome::Converted
            });
        }
        TrialEnd::Convert => {}
    }
    if !trial.free {
        return Ok(TrialOutcome::Converted);
    }
    let mut windows = db(SubscriptionPlanWindowRow::list_by_subscription(conn, row.id).await)?;
    windows.sort_by_key(|window| window.from_date);
    // A plan change ended the trial: the customer chose the plan it bills.
    if windows
        .iter()
        .skip(1)
        .any(|window| window.from_date == until)
    {
        return Ok(TrialOutcome::Converted);
    }
    let Some(version) = windows
        .iter()
        .rev()
        .find(|window| window.from_date <= until)
        .or(windows.first())
        .map(|window| window.plan_version_id)
    else {
        return Ok(TrialOutcome::Converted);
    };
    if crate::services::lifecycle::derived::plan_is_free_tx(conn, version, row.tenant_id).await?
        || !services
            .store
            .lacks_payment_method_tx(conn, row.tenant_id, row.id, row.charge_automatically)
            .await?
    {
        return Ok(TrialOutcome::Converted);
    }
    let without_card =
        TrialWithoutCard::parse(&trial.without_card, trial.without_card_plan_version_id)
            .unwrap_or_default();
    let (tenant_id, subscription_id) = (row.tenant_id, row.id);
    let written = match without_card {
        TrialWithoutCard::AwaitPayment => return Ok(TrialOutcome::AwaitingPayment),
        TrialWithoutCard::Cancel => services
            .store
            .transaction_with(conn, |conn| {
                async move {
                    crate::services::facts::record_end_date_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        Some(until),
                        actor,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
            .map(|()| TrialOutcome::Cancelled),
        TrialWithoutCard::Switch { plan_version_id } => services
            .store
            .transaction_with(conn, |conn| {
                async move {
                    write_switch_phase_tx(
                        services,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        until,
                        plan_version_id,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
            .map(|()| TrialOutcome::Switched),
    };
    Ok(written.unwrap_or_else(|error| {
        log::warn!(
            "Trial end of subscription {subscription_id} without a payment method left awaiting \
             payment: {error:?}"
        );
        TrialOutcome::AwaitingPayment
    }))
}
