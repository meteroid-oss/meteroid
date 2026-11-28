//! A running trial extended or ended today (R-153), its recorded end action moving with it; a
//! trial awaiting a decision converted, extended or cancelled (R-167).
use chrono::Datelike;
use common_domain::actor::Actor;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::subscription_facts::SubscriptionTrialRow;
use scoped_futures::ScopedFutureExt;

use super::{db, invalid};
use crate::StoreResult;
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::domain::subscription_phases::SubscriptionPhaseInput;
use crate::domain::trials::{MAX_TRIAL_DAYS, SubscriptionTrial, TrialEnd, TrialMove, TrialOutcome};
use crate::services::Services;

/// The subscription's trial: its fact, the one record of it (I29).
pub(crate) async fn trial_of_tx(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<Option<SubscriptionTrial>> {
    Ok(db(SubscriptionTrialRow::find(conn, subscription_id).await)?.map(Into::into))
}

impl Services {
    /// A free trial's billing starts on its new end, so it moves only while nothing else is
    /// scheduled from the old one.
    pub(in crate::services) async fn move_trial(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        to: TrialMove,
    ) -> StoreResult<SubscriptionTrial> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    self.move_trial_tx(conn, actor, tenant_id, subscription_id, to)
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    async fn move_trial_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        to: TrialMove,
    ) -> StoreResult<SubscriptionTrial> {
        use diesel_models::enums::SubscriptionChangeKindEnum as Kind;
        use diesel_models::subscription_changes::SubscriptionChangeRow;
        use diesel_models::subscriptions::SubscriptionRow;

        let lock = db(SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await)?;
        db(diesel_models::query::credit_inputs::bump_customer(conn, &lock).await)?;
        let subscription =
            db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
                .subscription;
        let trial = db(SubscriptionTrialRow::find(conn, subscription_id).await)?
            .ok_or_else(|| invalid("the subscription has no trial"))?;
        let today = common_clock::now().date_naive();
        let outcome = trial.outcome.as_deref().and_then(TrialOutcome::parse);
        let awaits_decision = outcome == Some(TrialOutcome::AwaitingDecision);
        if !awaits_decision && (outcome.is_some() || trial.until_date <= today) {
            return Err(invalid(format!("the trial ended on {}", trial.until_date)));
        }
        let end = TrialEnd::parse(&trial.end_action);
        if to == TrialMove::Cancel {
            if !awaits_decision {
                return Err(invalid(
                    "only a trial awaiting a decision is cancelled with it: cancel the subscription instead",
                ));
            }
            crate::services::facts::record_end_date_tx(
                conn,
                tenant_id,
                subscription_id,
                Some(today),
                actor,
            )
            .await?;
            return self
                .trial_moved_tx(conn, actor, tenant_id, &subscription, &trial)
                .await;
        }
        let until = match to {
            TrialMove::Until(day) => day,
            TrialMove::Days(days) => trial
                .until_date
                .checked_add_days(chrono::Days::new(u64::from(days)))
                .ok_or_else(|| invalid("the trial would end out of range"))?,
            TrialMove::Now | TrialMove::Cancel => today.max(trial.from_date),
        };
        if to != TrialMove::Now {
            if until <= trial.until_date.max(today) {
                return Err(invalid(format!(
                    "an extension ends after the trial's end, {}, and after today",
                    trial.until_date
                )));
            }
            if (until - trial.from_date).num_days() > i64::from(MAX_TRIAL_DAYS) {
                return Err(invalid(format!(
                    "a trial lasts {MAX_TRIAL_DAYS} days at most"
                )));
            }
        }
        let phases = crate::services::subscriptions::phases::list_phases_tx(
            conn,
            tenant_id,
            subscription_id,
        )
        .await?;
        let fallback_change = (end == TrialEnd::Fallback)
            .then(|| {
                phases
                    .iter()
                    .find(|phase| phase.start_date == trial.until_date)
                    .map(|phase| phase.change_id)
            })
            .flatten();
        if trial.free {
            let carried = crate::services::offers::follow::trial_bound_changes_tx(
                conn,
                tenant_id,
                subscription_id,
                trial.until_date,
            )
            .await?;
            let pending = db(SubscriptionChangeRow::list_pending(
                conn,
                &tenant_id,
                subscription_id,
                &[Kind::PlanChange, Kind::Amendment, Kind::Phase],
            )
            .await)?;
            if pending
                .iter()
                .any(|change| Some(change.id) != fallback_change && !carried.contains(&change.id))
            {
                return Err(invalid(
                    "changes are scheduled from the trial's end: withdraw them before moving it",
                ));
            }
        }
        // The switch written for the trial's end goes before the end moves (its plan window with
        // it), and is written again on the new end.
        let fallback_plan = phases
            .iter()
            .find(|phase| Some(phase.change_id) == fallback_change)
            .and_then(|phase| phase.terms.plan.as_ref())
            .map(|plan| plan.plan_version_id);
        if fallback_change.is_some() {
            if to == TrialMove::Now
                && phases
                    .iter()
                    .any(|phase| phase.start_date > trial.until_date)
            {
                return Err(invalid(
                    "phases follow the trial's switch: withdraw them before ending the trial now",
                ));
            }
            crate::services::subscriptions::phases::set_phases_tx(
                self,
                conn,
                actor,
                tenant_id,
                subscription_id,
                &[],
            )
            .await?;
        }
        crate::services::facts::record_trial_end_tx(conn, subscription_id, &trial, until).await?;
        // A free trial's periods start on its end: an anchor derived from the old end follows.
        if trial.free && i32::from(subscription.billing_day_anchor) == trial.until_date.day() as i32
        {
            let anchor = i16::try_from(until.max(trial.from_date).day())
                .unwrap_or(subscription.billing_day_anchor);
            db(SubscriptionRow::set_billing_day_anchor(conn, subscription_id, anchor).await)?;
        }
        if trial.free {
            crate::services::offers::follow::follow_trial_tx(
                self,
                conn,
                actor,
                tenant_id,
                subscription_id,
                subscription.current_period_start,
                trial.until_date,
                until,
            )
            .await?;
        }
        match end {
            TrialEnd::Convert | TrialEnd::Manual => {}
            TrialEnd::Cancel => {
                crate::services::facts::record_end_date_tx(
                    conn,
                    tenant_id,
                    subscription_id,
                    Some(until),
                    actor,
                )
                .await?;
            }
            TrialEnd::Fallback if to == TrialMove::Now => {
                if let Some(plan_version_id) = fallback_plan {
                    super::ending::write_switch_phase_tx(
                        self,
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        until,
                        plan_version_id,
                    )
                    .await?;
                }
            }
            TrialEnd::Fallback => {
                let moved: Vec<SubscriptionPhaseInput> = phases
                    .into_iter()
                    .filter(|phase| phase.start_date > today)
                    .map(|phase| SubscriptionPhaseInput {
                        start_date: if phase.start_date == trial.until_date {
                            until
                        } else {
                            phase.start_date
                        },
                        label: phase.label,
                        terms: phase.terms,
                    })
                    .collect();
                crate::services::subscriptions::phases::set_phases_tx(
                    self,
                    conn,
                    actor,
                    tenant_id,
                    subscription_id,
                    &moved,
                )
                .await?;
            }
        }
        let moved = db(SubscriptionTrialRow::find(conn, subscription_id).await)?;
        match (to, moved) {
            // Ended now, a manual trial converts: billing starts today.
            (TrialMove::Now, Some(moved)) if end == TrialEnd::Manual => {
                super::ending::record_tx(
                    conn,
                    &subscription,
                    &moved,
                    TrialOutcome::Converted,
                    until.and_time(chrono::NaiveTime::MIN),
                )
                .await?;
            }
            // Extended, a trial awaiting a decision runs again.
            (TrialMove::Until(_) | TrialMove::Days(_), Some(_)) if awaits_decision => {
                crate::services::facts::record_trial_outcome_tx(conn, subscription_id, None)
                    .await?;
            }
            _ => {}
        }
        self.trial_moved_tx(conn, actor, tenant_id, &subscription, &trial)
            .await
    }

    /// The trial's end decided when it is reached now (R-167), the state derived from the moved
    /// facts, and the trial as it reads after.
    async fn trial_moved_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription: &diesel_models::subscriptions::SubscriptionRow,
        before: &SubscriptionTrialRow,
    ) -> StoreResult<SubscriptionTrial> {
        let subscription_id = subscription.id;
        super::ending::decide_trial_end_tx(self, conn, actor, subscription, common_clock::now())
            .await?;
        self.derive_state_tx(conn, actor, tenant_id, subscription_id, common_clock::now())
            .await?;
        self.emit_subscription_updated_tx(
            conn,
            tenant_id,
            subscription_id,
            SubscriptionUpdateType::TrialChanged,
        )
        .await?;
        // Ended on the day it started, it never ran: read as a trial of no days.
        Ok(trial_of_tx(conn, subscription_id)
            .await?
            .unwrap_or_else(|| SubscriptionTrial {
                until: before.from_date,
                ..SubscriptionTrial::from(before.clone())
            }))
    }
}
