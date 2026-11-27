//! A running trial extended or ended today (R-153), its recorded end action moving with it.
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
use crate::domain::trials::{MAX_TRIAL_DAYS, SubscriptionTrial, TrialEnd, TrialMove};
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
        if trial.until_date <= today {
            return Err(invalid(format!("the trial ended on {}", trial.until_date)));
        }
        let end = TrialEnd::parse(&trial.end_action);
        let until = match to {
            TrialMove::Until(day) => day,
            TrialMove::Days(days) => trial
                .until_date
                .checked_add_days(chrono::Days::new(u64::from(days)))
                .ok_or_else(|| invalid("the trial would end out of range"))?,
            TrialMove::Now => today.max(trial.from_date),
        };
        if to == TrialMove::Now {
            if end != TrialEnd::Convert {
                return Err(invalid(
                    "a trial cancelling or switching plan at its end is not ended early: cancel or change the subscription instead",
                ));
            }
        } else if until <= trial.until_date {
            return Err(invalid(format!(
                "an extension ends after the trial's end, {}",
                trial.until_date
            )));
        } else if (until - trial.from_date).num_days() > i64::from(MAX_TRIAL_DAYS) {
            return Err(invalid(format!(
                "a trial lasts {MAX_TRIAL_DAYS} days at most"
            )));
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
            let pending = db(SubscriptionChangeRow::list_pending(
                conn,
                &tenant_id,
                subscription_id,
                &[Kind::PlanChange, Kind::Amendment, Kind::Phase],
            )
            .await)?;
            if pending
                .iter()
                .any(|change| Some(change.id) != fallback_change)
            {
                return Err(invalid(
                    "changes are scheduled from the trial's end: withdraw them before moving it",
                ));
            }
        }
        crate::services::facts::record_trial_end_tx(conn, subscription_id, &trial, until).await?;
        // A free trial's periods start on its end: an anchor derived from the old end follows.
        if trial.free && i32::from(subscription.billing_day_anchor) == trial.until_date.day() as i32
        {
            let anchor = i16::try_from(until.max(trial.from_date).day())
                .unwrap_or(subscription.billing_day_anchor);
            db(SubscriptionRow::set_billing_day_anchor(conn, subscription_id, anchor).await)?;
        }
        match end {
            TrialEnd::Convert => {}
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
        self.derive_state_tx(conn, actor, tenant_id, subscription_id, common_clock::now())
            .await?;
        self.emit_subscription_updated_tx(
            conn,
            tenant_id,
            subscription_id,
            SubscriptionUpdateType::TrialChanged,
        )
        .await?;
        Ok(trial_of_tx(conn, subscription_id)
            .await?
            .unwrap_or(SubscriptionTrial {
                from: trial.from_date,
                until: trial.from_date,
                free: trial.free,
                trial_plan_id: trial.trial_plan_id,
                offer_id: trial.trial_offer_id,
                end_action: end,
            }))
    }
}
