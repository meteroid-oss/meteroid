//! The derived state's plan concern, written from the facts: the plan version,
//! its cadence and the period it cuts, the MRR, the Switch row and the outbox event of a plan
//! transition, and when the facts next change any of it. One writer per column: the status,
//! `pending_checkout` and a renewal's new period stay the cycle's and the cancellation's, kept
//! apart by the two guards below.

use billing_core::evaluate::{Status, evaluate};
use billing_core::transition::{Deadlines, Step, Stored, Transition, step};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanVersionId, SubscriptionId, TenantId};
use diesel_models::enums::{CycleActionEnum, ScheduledEventTypeEnum, SubscriptionEventType};
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionCadenceRowPatch, SubscriptionRow};
use error_stack::Report;

use crate::StoreResult;
use crate::domain::SubscriptionStatusEnum;
use crate::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::errors::StoreError;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::services::Services;
use crate::services::evaluate::bill::midnight;
use crate::services::evaluate::load::load_facts;
use crate::services::evaluate::pricer::StorePricer;
use crate::services::invoice_lines::fees::pricing_error;
use crate::store::PgConn;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

/// The stored statuses a derived one stands for; none for those no fact derives yet.
pub(crate) fn stored_as(status: &SubscriptionStatusEnum) -> Option<Status> {
    match status {
        SubscriptionStatusEnum::PendingActivation | SubscriptionStatusEnum::PendingCharge => {
            Some(Status::Pending)
        }
        SubscriptionStatusEnum::TrialActive => Some(Status::Trialing),
        SubscriptionStatusEnum::TrialExpired => Some(Status::TrialExpired),
        SubscriptionStatusEnum::Active => Some(Status::Active),
        SubscriptionStatusEnum::Cancelled | SubscriptionStatusEnum::Aborted => {
            Some(Status::Cancelled)
        }
        SubscriptionStatusEnum::Completed => Some(Status::Completed),
        SubscriptionStatusEnum::Paused
        | SubscriptionStatusEnum::Suspended
        | SubscriptionStatusEnum::Superseded
        | SubscriptionStatusEnum::Errored => None,
    }
}

fn cadence_of(months: u32) -> Option<BillingPeriodEnum> {
    [
        BillingPeriodEnum::Monthly,
        BillingPeriodEnum::Quarterly,
        BillingPeriodEnum::Semiannual,
        BillingPeriodEnum::Annual,
    ]
    .into_iter()
    .find(|cadence| cadence.as_months() == months)
}

/// The instant the facts are read at: `now`, held inside the stored period until the renewal
/// advances it, so a change at its end is applied with the renewal that bills it (I27).
fn capped(now: DateTime<Utc>, current_period_end: Option<NaiveDate>) -> DateTime<Utc> {
    // PR 4: the cap goes once the writer owns the period too.
    match current_period_end {
        Some(end) => now.min(midnight(end) - Duration::seconds(1)),
        None => now,
    }
}

impl Services {
    /// Writes what the facts derive at `now` of the plan concern, under the customer's lock, in
    /// the caller's transaction: after a fact edit, inside a renewal, at the subscription's wake.
    /// Run again at the same instant, it writes nothing.
    pub(crate) async fn derive_state_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: DateTime<Utc>,
    ) -> StoreResult<Option<Step>> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(self.derive_state_unboxed(conn, actor, tenant_id, subscription_id, now)).await
    }

    async fn derive_state_unboxed(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        now: DateTime<Utc>,
    ) -> StoreResult<Option<Step>> {
        let row =
            db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
                .subscription;
        let status = SubscriptionStatusEnum::from(row.status.clone());
        let live = matches!(
            status,
            SubscriptionStatusEnum::PendingActivation
                | SubscriptionStatusEnum::PendingCharge
                | SubscriptionStatusEnum::TrialActive
                | SubscriptionStatusEnum::TrialExpired
                | SubscriptionStatusEnum::Active
        );
        let Some(stored_status) = stored_as(&status).filter(|_| live) else {
            if row.next_wake_at.is_some() {
                db(SubscriptionRow::set_next_wake_at(conn, subscription_id, None).await)?;
            }
            return Ok(None);
        };

        let at = capped(now, row.current_period_end);
        let loaded = load_facts(conn, tenant_id, subscription_id, at.date_naive(), None).await?;
        let evaluation = evaluate(&loaded.facts, at, None, &StorePricer).map_err(pricing_error)?;
        let stored_period = BillingPeriodEnum::from(row.period.clone());
        let stored = Stored {
            status: stored_status,
            current_period: Some((row.current_period_start, row.current_period_end)),
            plan_version: Some(row.plan_version_id),
            cadence: Some(stored_period.as_months()),
            pending_checkout: row.pending_checkout,
        };
        let deadlines = Deadlines {
            finalize_after: &[],
            next_payment_attempt_at: &[],
            credit_lapses: &[],
            settlement_grace: Duration::zero(),
        };
        let step = step(&loaded.facts, at, &evaluation, &stored, &[], &deadlines);
        let derived = &evaluation.state;

        // PR 4: the scope widens to the status transitions, written by the cycle until then.
        let switches: Vec<(PlanVersionId, PlanVersionId, NaiveDate)> = step
            .transitions
            .iter()
            .filter_map(|transition| match transition {
                Transition::Plan { from, to, at } => Some((*from, *to, at.date_naive())),
                Transition::Status { .. } => None,
            })
            .collect();
        for (from, to, on) in &switches {
            self.record_switch_tx(conn, actor, &row, *from, *to, *on, &stored_period)
                .await?;
        }
        let amended = self.record_amendments_tx(conn, &row, at).await?;

        let plan_version = derived.plan_version.unwrap_or(row.plan_version_id);
        let cadence = derived
            .cadence
            .and_then(cadence_of)
            .unwrap_or(stored_period);
        let changed = plan_version != row.plan_version_id || cadence != stored_period;
        if changed {
            db(SubscriptionRow::update_plan_version(
                conn,
                &subscription_id,
                &tenant_id,
                plan_version,
                Some(cadence.into()),
            )
            .await)?;
        }
        // The period the new cadence cuts, inside the stored one (it replaces the realignment).
        if cadence != stored_period
            && let Some((start, Some(end))) = derived.current_period
            && row
                .current_period_end
                .is_some_and(|stored_end| start < stored_end)
            && (start, Some(end)) != (row.current_period_start, row.current_period_end)
        {
            let next_cycle_action = match row.end_date {
                Some(end_date) if end_date <= end => CycleActionEnum::EndSubscription,
                _ => CycleActionEnum::RenewSubscription,
            };
            // A fee's cadence moved it (no plan switch): its cycles count on the new grid.
            let cycle_index = switches.is_empty().then(|| {
                crate::utils::periods::calculate_elapsed_cycles(
                    row.billing_start_date.unwrap_or(row.current_period_start),
                    at.date_naive(),
                    &cadence,
                    u32::from(row.billing_day_anchor.unsigned_abs()),
                ) as i32
            });
            db(SubscriptionCadenceRowPatch {
                id: subscription_id,
                tenant_id,
                period: cadence.into(),
                current_period_start: start,
                current_period_end: Some(row.end_date.map_or(end, |end_date| end_date.min(end))),
                cycle_index,
                next_cycle_action: Some(next_cycle_action),
            }
            .patch(conn)
            .await)?;
        }

        // The fees in force at the instant read: a change at the period's end moves the MRR
        // with the renewal that applies it.
        let on = switches
            .last()
            .map(|(_, _, on)| *on)
            .or_else(|| latest_change(&loaded.facts, at.date_naive()))
            .unwrap_or(at.date_naive());
        self.restate_mrr_tx(conn, tenant_id, subscription_id, at.date_naive(), on)
            .await?;

        let wake = step.next_wake_at.filter(|wake| *wake > now);
        if wake != row.next_wake_at {
            db(SubscriptionRow::set_next_wake_at(conn, subscription_id, wake).await)?;
        }
        if plan_version != row.plan_version_id || !switches.is_empty() {
            self.emit_subscription_updated_tx(
                conn,
                tenant_id,
                subscription_id,
                SubscriptionUpdateType::PlanChanged,
            )
            .await?;
        } else if amended {
            self.emit_subscription_updated_tx(
                conn,
                tenant_id,
                subscription_id,
                SubscriptionUpdateType::Amended,
            )
            .await?;
        }
        Ok(Some(step))
    }

    /// The Switch row of each amendment written ahead that took effect by `at`, once; true when
    /// one took effect since the last run, read from the change's MIGRATED record.
    async fn record_amendments_tx(
        &self,
        conn: &mut PgConn,
        row: &SubscriptionRow,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let in_force = db(ScheduledEventRow::list_migrated_in_force(
            conn,
            &row.tenant_id,
            row.id,
            ScheduledEventTypeEnum::ApplyAmendment,
            at.naive_utc(),
        )
        .await)?;
        let mut amended = false;
        for event in in_force {
            if db(SubscriptionEventRow::change_applied(conn, row.id, event.id).await)? {
                continue;
            }
            db(SubscriptionEventRow {
                id: uuid::Uuid::now_v7(),
                subscription_id: row.id,
                event_type: SubscriptionEventType::Switch,
                details: Some(serde_json::json!({
                    "kind": "amendment",
                    "mode": "end_of_period",
                    "event": event.id.as_uuid().to_string(),
                })),
                created_at: common_clock::now().naive_utc(),
                mrr_delta: None,
                bi_mrr_movement_log_id: None,
                applies_to: event.scheduled_time.date(),
            }
            .insert(conn)
            .await)?;
            // A longer period of a fee it changed gives back what was billed past its day, before
            // the renewal of that day bills.
            self.settle_scheduled_amendment_tx(
                conn,
                &Actor::System,
                row.tenant_id,
                row.id,
                event.scheduled_time.date(),
            )
            .await?;
            let activity = Activity::new(
                ActivityType::SubscriptionAmended,
                EntityType::Subscription,
                row.id.as_uuid(),
            )
            .with_metadata(serde_json::json!({
                "effective_date": event.scheduled_time.date().to_string(),
            }))
            .agg_customer(row.customer_id);
            self.store
                .record_tx(
                    conn,
                    row.tenant_id,
                    &Actor::System,
                    AuditInput::Activity(activity),
                )
                .await?;
            amended = true;
        }
        Ok(amended)
    }

    /// A plan transition's Switch row, signal and activity, once per (subscription, plan, day).
    #[allow(clippy::too_many_arguments)]
    async fn record_switch_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        row: &SubscriptionRow,
        from: PlanVersionId,
        to: PlanVersionId,
        on: NaiveDate,
        outgoing_period: &BillingPeriodEnum,
    ) -> StoreResult<()> {
        if db(SubscriptionEventRow::switch_recorded(conn, row.id, to, on).await)? {
            return Ok(());
        }
        // The period ending here is trued up under the terms it ran on.
        let outgoing_minimum = crate::repositories::subscriptions::load_subscription_minimum_on(
            conn,
            row.id,
            on.pred_opt().unwrap_or(on),
        )
        .await?;
        let mut details = serde_json::json!({
            "new_plan_version_id": to.to_string(),
            "old_plan_version_id": from.to_string(),
        });
        // The change written ahead it applies, no longer pending once named here.
        let scheduled = db(ScheduledEventRow::list_migrated_in_force(
            conn,
            &row.tenant_id,
            row.id,
            ScheduledEventTypeEnum::ApplyPlanChange,
            on.and_time(chrono::NaiveTime::MIN),
        )
        .await)?
        .into_iter()
        .find(|event| {
            event.scheduled_time.date() == on
                && event
                    .event_data
                    .pointer("/ApplyPlanChange/new_plan_version_id")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|id| id.parse::<PlanVersionId>().ok())
                    == Some(to)
        });
        if let Some(event) = scheduled {
            details["event"] = serde_json::Value::String(event.id.as_uuid().to_string());
        }
        for (key, value) in crate::services::invoice_lines::minimum::outgoing_terms_record(
            outgoing_minimum.as_ref(),
            *outgoing_period,
        ) {
            details[key] = value;
        }
        db(SubscriptionEventRow {
            id: uuid::Uuid::now_v7(),
            subscription_id: row.id,
            event_type: SubscriptionEventType::Switch,
            details: Some(details),
            created_at: common_clock::now().naive_utc(),
            mrr_delta: None,
            bi_mrr_movement_log_id: None,
            applies_to: on,
        }
        .insert(conn)
        .await)?;
        self.store
            .hooks
            .signals
            .subscription_plan_changed(
                conn,
                row.tenant_id,
                row.customer_id,
                row.id,
                common_clock::now().naive_utc(),
            )
            .await?;
        let activity = Activity::new(
            ActivityType::SubscriptionPlanChanged,
            EntityType::Subscription,
            row.id.as_uuid(),
        )
        .with_metadata(serde_json::json!({
            "new_plan_version_id": to.to_string(),
            "change_date": on.to_string(),
        }))
        .agg_customer(row.customer_id);
        self.store
            .record_tx(conn, row.tenant_id, actor, AuditInput::Activity(activity))
            .await
    }

    /// The MRR of the fees in force on `day`, its movement booked on `on`. Before the first
    /// period's New Business is booked the stored MRR is not the subscription's yet: nothing moves.
    async fn restate_mrr_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        day: NaiveDate,
        on: NaiveDate,
    ) -> StoreResult<()> {
        let row =
            db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
                .subscription;
        if !crate::services::credit::coupon_grant::has_started(&row) {
            return Ok(());
        }
        let created = db(
            SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
                conn,
                subscription_id,
                SubscriptionEventType::Created,
                row.start_date,
            )
            .await,
        )?;
        if created.is_some_and(|event| {
            event.mrr_delta.is_some_and(|delta| delta != 0)
                && event.bi_mrr_movement_log_id.is_none()
        }) {
            return Ok(());
        }
        let mut details = self
            .store
            .subscription_details_as_of(conn, tenant_id, subscription_id, day)
            .await?;
        let precision = crate::constants::Currencies::resolve_currency_precision(
            &details.subscription.currency,
        )
        .unwrap_or(2);
        let lines =
            crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots(
                conn,
                tenant_id,
                subscription_id,
                &details.price_components,
                precision,
            )
            .await?;
        let mrr = self
            .restate_subscription_mrr(conn, &mut details, lines, precision, on)
            .await?;
        let delta = mrr - details.subscription.mrr_cents as i64;
        if delta != 0 {
            db(SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, delta).await)?;
            self.create_movement_mrr_log(conn, tenant_id, subscription_id, on, delta)
                .await?;
        }
        Ok(())
    }
}

/// The latest day on or before `today` a fee row, a seat step or a minimum started or ended on:
/// the change an MRR movement belongs to.
fn latest_change<F>(facts: &billing_core::facts::Facts<F>, today: NaiveDate) -> Option<NaiveDate> {
    facts
        .fees
        .iter()
        .flat_map(|fee| [Some(fee.from), fee.to])
        .chain(facts.seats.iter().map(|seat| Some(seat.at)))
        .chain(
            facts
                .minimums
                .iter()
                .flat_map(|minimum| [Some(minimum.from), minimum.to]),
        )
        .flatten()
        .filter(|day| *day <= today)
        .max()
}
