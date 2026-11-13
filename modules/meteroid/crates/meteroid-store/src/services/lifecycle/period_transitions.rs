use crate::StoreResult;
use crate::domain::DetailedInvoice;
use crate::errors::{StoreError, StoreErrorContainer};
use crate::services::{InvoiceBillingMode, InvoiceDelivery, Services};
use crate::store::PgConn;
use crate::utils::errors::format_error_chain;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{DateTime, Days, Duration, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::{CustomerId, SubscriptionId, TenantId};
use diesel_async::AsyncConnection;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{
    CycleActionEnum, PlanTypeEnum, ScheduledEventTypeEnum, SubscriptionActivationConditionEnum,
    SubscriptionStatusEnum,
};
use diesel_models::plans::PlanRow;
use diesel_models::query::credit_inputs;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscriptions::{
    SubscriptionCycleErrorRowPatch, SubscriptionCycleRowPatch, SubscriptionRow,
};
use error_stack::Report;
use futures::stream::StreamExt;
use scoped_futures::ScopedFutureExt;

const BATCH_SIZE: i64 = 8;
const MAX_CYCLE_RETRIES: i32 = 10;
const MAX_PARALLEL_PROCESSING: usize = 4;
const BUSY_CUSTOMER_RETRY_SECONDS: std::ops::RangeInclusive<i64> = 30..=90;

enum Visit {
    Done,
    CustomerBusy,
}

/// Result of processing cycle transitions
pub struct CycleTransitionResult {
    /// Number of subscriptions claimed for processing
    pub processed: usize,
    /// Whether there may be more work (processed == batch_size)
    pub has_more: bool,
}

impl Services {
    /// Claims and processes due subscription cycle transitions.
    ///
    /// Uses a claim-then-process pattern for parallelism:
    /// 1. Short transaction to claim subscriptions (set processing_started_at)
    /// 2. Process each subscription in parallel with its own transaction
    /// 3. On success, the subscription advances; on failure, error is recorded
    pub async fn get_and_process_cycle_transitions(&self) -> StoreResult<CycleTransitionResult> {
        // Phase 1: Claim subscription IDs (short transaction)
        let claimed_ids = self
            .store
            .transaction(|tx| {
                async move {
                    SubscriptionRow::claim_due_subscriptions(tx, BATCH_SIZE)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                }
                .scope_boxed()
            })
            .await?;

        let claimed_count = claimed_ids.len();

        if claimed_count == 0 {
            return Ok(CycleTransitionResult {
                processed: 0,
                has_more: false,
            });
        }

        // Phase 2: Process each customer's subscriptions in turn, customers in parallel: two
        // renewals of one customer would otherwise find it busy and skip each other.
        let mut by_customer: Vec<(CustomerId, TenantId, Vec<SubscriptionId>)> = Vec::new();
        for (subscription_id, customer_id, tenant_id) in claimed_ids {
            match by_customer.iter_mut().find(|(c, _, _)| *c == customer_id) {
                Some((_, _, ids)) => ids.push(subscription_id),
                None => by_customer.push((customer_id, tenant_id, vec![subscription_id])),
            }
        }
        let results: Vec<_> = futures::stream::iter(by_customer)
            .map(|(customer_id, tenant_id, ids)| async move {
                let mut visits = Vec::with_capacity(ids.len());
                for id in ids {
                    let result = self
                        .process_subscription_cycle(tenant_id, customer_id, id)
                        .await;
                    visits.push(((tenant_id, customer_id, id), result));
                }
                futures::stream::iter(visits)
            })
            .buffer_unordered(MAX_PARALLEL_PROCESSING)
            .flatten()
            .collect()
            .await;

        // Log any errors (they're already handled per-subscription)
        let mut skipped = 0;
        for ((tenant_id, customer_id, subscription_id), result) in &results {
            match result {
                Ok(Visit::CustomerBusy) => skipped += 1,
                Ok(Visit::Done) => {}
                Err(e) => tracing::error!(
                    tenant_id = %tenant_id,
                    customer_id = %customer_id,
                    subscription_id = %subscription_id,
                    "Unexpected error in subscription processing: {e:?}"
                ),
            }
        }

        // Busy customers' subscriptions are deferred, so claiming again never finds them first.
        Ok(CycleTransitionResult {
            processed: claimed_count - skipped,
            has_more: claimed_count == BATCH_SIZE as usize,
        })
    }

    /// Lock and processes a single subscription in its own transaction.
    /// Uses a savepoint for the actual processing to record the error on failure:
    #[tracing::instrument(skip_all, fields(
        tenant_id = %tenant_id,
        customer_id = %customer_id,
        subscription_id = %subscription_id,
    ))]
    async fn process_subscription_cycle(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Visit> {
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                // Behind its customer (the lock order), never waiting on it, in every pass: a busy
                // customer defers the subscription, which is not a failure. Deferred, it no longer
                // heads the oldest-due claims, so busy customers cannot stall the others.
                let Some(lock) = SubscriptionRow::try_lock_customer_of(conn, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                else {
                    SubscriptionRow::defer_claim_unless_locked(
                        conn,
                        subscription_id,
                        busy_customer_retry_time(),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    return Ok(Visit::CustomerBusy);
                };
                let Some(subscription) =
                    SubscriptionRow::get_and_lock_for_processing(conn, &lock, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                else {
                    log::debug!(
                        "Subscription {} no longer due for processing, skipping",
                        subscription_id
                    );
                    return Ok(Visit::Done);
                };

                let tenant_id = subscription.tenant_id;
                let subscription_id = subscription.id;
                let error_count = subscription.error_count;

                // A change dated inside the period (in retry backoff, or waiting on metering) applies
                // first: renewed before it, the next period would be billed and never credited.
                if let Some(next_attempt) = unapplied_change_of_period(conn, &subscription).await? {
                    let now = common_domain::clock::now().naive_utc();
                    let retry_at = next_attempt.filter(|at| *at > now).unwrap_or(now)
                        + Duration::minutes(1);
                    log::warn!(
                        "Subscription {subscription_id} waits for a change dated in its period, until {retry_at}"
                    );
                    SubscriptionRow::release_claim(conn, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    SubscriptionCycleErrorRowPatch {
                        id: subscription_id,
                        tenant_id,
                        last_error: Some(Some(UNAPPLIED_CHANGE.to_owned())),
                        next_retry: Some(Some(retry_at)),
                        error_count: None,
                        status: None,
                    }
                    .patch(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    return Ok(Visit::Done);
                }

                // Try processing in a savepoint (nested transaction)
                let process_result = conn
                    .transaction(async |inner_conn| {
                        services
                            .process_cycle_transition(inner_conn, &subscription, None)
                            .await
                            .map(|_| ())
                            .map_err(Into::<StoreErrorContainer>::into)
                    })
                    .await;

                // Handle result while still holding the lock
                match process_result {
                    Ok(()) => {
                        // Clear error state
                        SubscriptionCycleErrorRowPatch {
                            id: subscription_id,
                            tenant_id,
                            last_error: Some(None),
                            next_retry: Some(None),
                            error_count: Some(0),
                            status: None,
                        }
                        .patch(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    Err(err) if super::waits_on_metering(&err.error) => {
                        services.ensure_usage_served()?;
                        log::warn!(
                            "Subscription {subscription_id} waits on the metering service: {}",
                            format_error_chain(&err.error)
                        );
                        SubscriptionCycleErrorRowPatch {
                            id: subscription_id,
                            tenant_id,
                            last_error: Some(Some(StoreError::MeteringNotReady.to_string())),
                            next_retry: Some(Some(super::metering_retry_time())),
                            error_count: None,
                            status: None,
                        }
                        .patch(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    Err(err) => {
                        // A failure on metering this pass was not served is priced again.
                        services.ensure_usage_served()?;
                        // Record error (savepoint already rolled back)
                        let error_message = format_error_chain(&err.error);
                        let new_error_count = error_count + 1;

                        let (status, next_retry) = if new_error_count >= MAX_CYCLE_RETRIES {
                            log::error!(
                                "Subscription {} exceeded max retries ({}), marking as Errored: {}",
                                subscription_id,
                                MAX_CYCLE_RETRIES,
                                error_message
                            );
                            (Some(SubscriptionStatusEnum::Errored), Some(None))
                        } else {
                            log::warn!(
                                "Error processing subscription {} (attempt {}/{}): {}",
                                subscription_id,
                                new_error_count,
                                MAX_CYCLE_RETRIES,
                                error_message
                            );
                            (None, Some(Some(calculate_retry_time(new_error_count))))
                        };

                        SubscriptionCycleErrorRowPatch {
                            id: subscription_id,
                            tenant_id,
                            last_error: Some(Some(error_message)),
                            next_retry,
                            error_count: Some(new_error_count),
                            status,
                        }
                        .patch(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        // Exhausting retries makes the subscription terminal (Errored, never
                        // reclaimed); mirror a completion so its active-count +1 is churned.
                        if new_error_count >= MAX_CYCLE_RETRIES {
                            services
                                .churn_ended_subscription(
                                    conn,
                                    &subscription,
                                    common_domain::clock::now().naive_utc().date(),
                                )
                                .await?;
                        }
                    }
                }
                credit_inputs::bump_customer(conn, &lock)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(Visit::Done)
            }
            .scope_boxed()
        })
        .await
    }

    /// Runs, in one transaction, the cycle transition of a subscription whose period ends by
    /// `issued_at`, settling the period it bills inline and issuing its invoice at that moment
    /// instead of after the grace period. Errors roll everything back and are not retried.
    pub(crate) async fn process_cycle_transition_at(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        issued_at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<Option<DetailedInvoice>> {
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                services
                    .process_cycle_transition_at_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        issued_at,
                        delivery,
                    )
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    /// `process_cycle_transition_at` in the caller's transaction.
    pub(in crate::services) async fn process_cycle_transition_at_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        issued_at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<Option<DetailedInvoice>> {
        if issued_at > common_domain::clock::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a subscription period cannot be billed at a future moment".to_owned(),
            )));
        }
        let lock = SubscriptionRow::lock_customer_of(conn, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let subscription = SubscriptionRow::lock_due_by(
            conn,
            &lock,
            tenant_id,
            subscription_id,
            issued_at.date_naive(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "the subscription has no period ending by then, or is being processed".to_owned(),
            ))
        })?;
        if unapplied_change_of_period(conn, &subscription)
            .await?
            .is_some()
        {
            return Err(Report::new(StoreError::FailedPrecondition(
                UNAPPLIED_CHANGE.to_owned(),
            )));
        }
        let billed = self
            .process_cycle_transition(conn, &subscription, Some(issued_at))
            .await?;
        credit_inputs::bump_customer(conn, &lock)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        SubscriptionCycleErrorRowPatch {
            id: subscription.id,
            tenant_id,
            last_error: Some(None),
            next_retry: Some(None),
            error_count: Some(0),
            status: None,
        }
        .patch(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        if let Some(invoice) = &billed {
            delivery
                .apply_tx(conn, tenant_id, invoice.invoice.id)
                .await?;
        }
        Ok(billed)
    }

    /// Executes the current scheduled action for a subscription.
    /// (activate, renew, end trial, end subscription, etc.)
    /// With `settle_at`, the period it bills settles inline, issued at that moment.
    async fn process_cycle_transition(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
        settle_at: Option<DateTime<Utc>>,
    ) -> StoreResult<Option<DetailedInvoice>> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            // filter terminal states just in case
            let next_action = if let Some(action) = &subscription.next_cycle_action {
                action
            } else {
                log::warn!(
                    "No next cycle action for subscription {}. Skipping processing.",
                    subscription.id
                );
                return Ok(None);
            };

            let mut paid_trial_days = None;
            let mut next_cycle = match next_action {
                CycleActionEnum::ActivateSubscription => {
                    let (next_cycle, paid_trial) =
                        self.activate_subscription(conn, subscription).await?;
                    paid_trial_days = paid_trial;
                    next_cycle
                }
                CycleActionEnum::RenewSubscription => self.renew_subscription(subscription)?,
                CycleActionEnum::EndTrial => {
                    // Trial ends - determine next state based on plan type and payment method
                    self.end_trial(conn, subscription).await?
                }
                CycleActionEnum::EndSubscription => self.end_subscription(subscription)?,
            };

            if let Some(end_date) = subscription.end_date.filter(|end_date| {
                next_cycle
                    .new_period_end
                    .is_some_and(|new_period_end| end_date <= &new_period_end)
            }) {
                next_cycle.next_cycle_action = Some(CycleActionEnum::EndSubscription);
                next_cycle.new_period_end = Some(end_date);
                // end_date is before the new period even starts: the subscription already ended,
                // nothing to bill for this period.
                if end_date <= next_cycle.new_period_start {
                    next_cycle.should_bill = false;
                }
            }

            // Before an event applied now or the renewal changes the period it is read from.
            if matches!(
                next_action,
                CycleActionEnum::RenewSubscription | CycleActionEnum::EndSubscription
            ) {
                let lock = SubscriptionRow::lock_customer_of(conn, subscription.id).await?;
                crate::services::invoice_lines::cadence::record_cadence_origins_tx(
                    conn,
                    &lock,
                    crate::services::invoice_lines::cadence::CadenceSubject::of_row(subscription),
                    next_cycle.new_period_start,
                )
                .await?;
            }

            // if subscription ended, we don't consider the other terminal states
            if next_cycle.status != SubscriptionStatusEnum::Completed {
                // check if we have an event today that would terminate the subscription
                let next_event = ScheduledEventRow::get_event_by_types_and_date_for_update(
                    conn,
                    subscription.id,
                    &subscription.tenant_id,
                    vec![
                        ScheduledEventTypeEnum::CancelSubscription,
                        ScheduledEventTypeEnum::ApplyPlanChange,
                        ScheduledEventTypeEnum::ApplyAmendment,
                        ScheduledEventTypeEnum::PauseSubscription,
                    ],
                    next_cycle.new_period_start,
                )
                .await?;

                if let Some(event) = next_event {
                    log::trace!(
                        "Found next event for subscription {}: {:?}",
                        subscription.id,
                        event
                    );
                    // Non-terminal events (plan change, amendment) apply at period end and then
                    // billing continues; terminal events (cancel, pause) stop the cycle.
                    let is_non_terminal = matches!(
                        event.event_type,
                        ScheduledEventTypeEnum::ApplyPlanChange
                            | ScheduledEventTypeEnum::ApplyAmendment
                    );
                    let event_id = event.id;

                    ScheduledEventRow::mark_as_processing(conn, &[event_id]).await?;
                    self.process_event_batch(conn, vec![event]).await?;
                    ScheduledEventRow::mark_as_completed(conn, &[event_id]).await?;

                    if !is_non_terminal {
                        // Terminal events (cancel, pause): clear processing claim and stop.
                        SubscriptionCycleRowPatch {
                            id: subscription.id,
                            tenant_id: subscription.tenant_id,
                            status: None,
                            next_cycle_action: None,
                            current_period_start: None,
                            current_period_end: None,
                            cycle_index: None,
                            pending_checkout: None,
                            processing_started_at: Some(None),
                            billing_start_date: None,
                            billing_day_anchor: None,
                            terminated_at: None,
                        }
                        .patch(conn)
                        .await?;

                        return Ok(None);
                    }
                    // Plan change / amendment: fall through to advance period and bill at new prices.
                }
            }

            // A renewal after a change of cadence ends on the new cadence's grid, from the first
            // renewal on it: a switch applied just now, or one an earlier release ended off grid.
            // The end date then cuts the realigned period, not the one computed before the switch.
            if matches!(next_action, CycleActionEnum::RenewSubscription)
                && next_cycle.new_period_end.is_some()
                && let Some(run) = crate::services::invoice_lines::cadence::cadence_runs_of(
                    conn,
                    crate::services::invoice_lines::cadence::CadenceSubject::of_row(subscription),
                    next_cycle.new_period_start,
                    true,
                )
                .await?
                .into_iter()
                .next()
            {
                let cadence: crate::domain::enums::BillingPeriodEnum =
                    SubscriptionRow::get_subscription_by_id(
                        conn,
                        &subscription.tenant_id,
                        subscription.id,
                    )
                    .await?
                    .subscription
                    .period
                    .into();
                if let Some(end) = run.period_end_after(
                    next_cycle.new_period_start,
                    cadence.as_months(),
                    subscription.billing_day_anchor as u32,
                ) {
                    match subscription.end_date.filter(|end_date| *end_date <= end) {
                        Some(end_date) => {
                            next_cycle.next_cycle_action = Some(CycleActionEnum::EndSubscription);
                            next_cycle.new_period_end = Some(end_date);
                        }
                        None => {
                            next_cycle.next_cycle_action = Some(CycleActionEnum::RenewSubscription);
                            next_cycle.new_period_end = Some(end);
                        }
                    }
                }
            }

            // Calculate new cycle_index:
            // - Only increment on RenewSubscription (actual renewal to new billing period)
            // - EndTrial starts billing but stays in cycle 0 (same period)
            // - Other transitions keep existing value
            let new_cycle_index = match next_action {
                CycleActionEnum::RenewSubscription => {
                    // Renewal always increments the cycle
                    match subscription.cycle_index {
                        Some(i) => Some(i + 1),
                        // Only a subscription the lifecycle activated before activation opened
                        // cycle 0 renews with none: the period it ends was cycle 0.
                        None => Some(1),
                    }
                }
                CycleActionEnum::EndTrial => {
                    // Trial end starts billing but doesn't increment cycle
                    // (we're still in cycle 0, just now with billing)
                    subscription.cycle_index
                }
                // A subscription that starts later had no cycle until now: activation opens cycle 0,
                // so its first renewal bills the first period's arrears as cycle 1.
                CycleActionEnum::ActivateSubscription => {
                    Some(subscription.cycle_index.unwrap_or(0))
                }
                _ => subscription.cycle_index,
            };

            // Capture before `status`/`next_cycle` fields are moved into the patch below.
            let is_completion = next_cycle.status == SubscriptionStatusEnum::Completed;
            let is_trial_end = matches!(next_action, CycleActionEnum::EndTrial);

            let patch = SubscriptionCycleRowPatch {
                id: subscription.id,
                tenant_id: subscription.tenant_id,
                status: Some(next_cycle.status),
                next_cycle_action: Some(next_cycle.next_cycle_action),
                current_period_start: Some(next_cycle.new_period_start),
                current_period_end: Some(next_cycle.new_period_end),
                cycle_index: new_cycle_index,
                pending_checkout: Some(next_cycle.pending_checkout),
                // Clear claim so subscription can be picked up again if still due
                processing_started_at: Some(None),
                billing_start_date: None,
                billing_day_anchor: None,
                terminated_at: None,
            };

            let completed_on = next_cycle.new_period_start;
            crate::services::subscriptions::start::apply_cycle_transition_tx(
                conn,
                &*self.store.hooks.credit,
                subscription,
                patch,
                settle_at.unwrap_or_else(common_domain::clock::now),
            )
            .await?;

            // A change applied at the period end may have changed the cadence its fees renew on.
            if matches!(next_action, CycleActionEnum::RenewSubscription) && !is_completion {
                crate::services::subscriptions::cadence::realign_cadence_tx(
                    conn,
                    subscription.tenant_id,
                    subscription.id,
                    completed_on,
                )
                .await?;
            }

            // A paid trial bills like any period; only its status changes when the trial ends.
            if let Some(trial_days) = paid_trial_days {
                use crate::repositories::SubscriptionInterface;
                let end_trial = crate::domain::scheduled_events::ScheduledEventNew::end_trial(
                    subscription.id,
                    subscription.tenant_id,
                    completed_on,
                    trial_days,
                    "subscription_activation",
                )
                .ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "Failed to compute trial end date".to_string(),
                    ))
                })?;
                self.store.schedule_events(conn, vec![end_trial]).await?;
            }

            // Before the final invoice, so its settlement already sees the credit lapsing.
            if is_completion {
                crate::services::credit::subscription_end::end_subscription_credit_tx(
                    conn,
                    &*self.store.hooks.credit,
                    subscription.tenant_id,
                    subscription.id,
                    completed_on.and_time(chrono::NaiveTime::MIN).and_utc(),
                    common_domain::clock::now(),
                )
                .await?;
            }

            let billed = if next_cycle.should_bill {
                self.bill_subscription_tx(
                    conn,
                    &crate::domain::entity_activity::Actor::System,
                    subscription.tenant_id,
                    subscription.id,
                    settle_at.map_or(
                        InvoiceBillingMode::AwaitGracePeriodIfApplicable,
                        InvoiceBillingMode::SettleAt,
                    ),
                )
                .await?
            } else {
                None
            };
            if next_cycle.should_bill
                && matches!(next_action, CycleActionEnum::ActivateSubscription)
            {
                self.book_created_mrr_tx(conn, subscription.tenant_id, subscription.id)
                    .await?;
            }

            // A fixed-term subscription that reached its end date churns its MRR: the
            // completion path emits no scheduled event, so the terminal Ended event and the
            // churn movement are booked here (mirrors a cancellation churn).
            if is_completion {
                self.churn_ended_subscription(conn, subscription, next_cycle.new_period_start)
                    .await?;
                // Dedicated terminal event, sibling of `subscription.cancelled`.
                self.emit_subscription_ended_tx(conn, subscription.tenant_id, subscription.id)
                    .await?;
            } else if is_trial_end {
                // Free-trial end (paid trials transition via the scheduled EndTrial handler instead).
                self.emit_subscription_updated_tx(
                    conn,
                    subscription.tenant_id,
                    subscription.id,
                    crate::domain::outbox_event::SubscriptionUpdateType::TrialEnded,
                )
                .await?;
            }

            Ok(billed)
        })
        .await
    }

    /// Starts a subscription on its start date. A paid trial bills its first period now, like a
    /// subscription without a trial, and returns its length for the event ending it; a free trial
    /// bills nothing until it ends.
    async fn activate_subscription(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
    ) -> StoreResult<(NextCycle, Option<i32>)> {
        if let Some(trial_duration) = subscription.trial_duration {
            let new_period_start = subscription
                .current_period_end
                .unwrap_or_else(|| common_domain::clock::now().naive_utc().date());
            let trial_is_free = PlanRow::get_with_version(
                conn,
                subscription.plan_version_id,
                subscription.tenant_id,
            )
            .await?
            .version
            .is_none_or(|version| version.trial_is_free);
            if !trial_is_free {
                let period = calculate_advance_period_range(
                    new_period_start,
                    subscription.billing_day_anchor as u32,
                    true,
                    &(subscription.period.clone().into()),
                );
                return Ok((
                    NextCycle {
                        status: SubscriptionStatusEnum::TrialActive,
                        next_cycle_action: Some(CycleActionEnum::RenewSubscription),
                        new_period_start,
                        new_period_end: Some(period.end),
                        should_bill: true,
                        pending_checkout: false,
                    },
                    Some(trial_duration),
                ));
            }
            let new_period_end = new_period_start
                .checked_add_days(Days::new(trial_duration as u64))
                .unwrap_or_else(|| new_period_start + Duration::days(7));
            Ok((
                NextCycle {
                    status: SubscriptionStatusEnum::TrialActive,
                    next_cycle_action: Some(CycleActionEnum::EndTrial),
                    new_period_start,
                    new_period_end: Some(new_period_end),
                    should_bill: false,
                    pending_checkout: false,
                },
                None,
            ))
        } else {
            Ok((self.renew_subscription(subscription)?, None))
        }
    }

    /// Handles the end of a FREE trial period (via CycleActionEnum::EndTrial).
    ///
    /// NOTE: Paid trials do NOT use this code path. They use:
    /// - CycleActionEnum::RenewSubscription for normal billing cycles
    /// - ScheduledEventTypeEnum::EndTrial to transition TrialActive → Active
    ///
    /// Business logic for FREE trials:
    /// - Free plan: → Active (no invoice ever)
    /// - Free trial on paid plan + has payment method: → Active + invoice
    /// - Free trial on paid plan + no payment method: → TrialExpired (awaiting checkout)
    async fn end_trial(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
    ) -> StoreResult<NextCycle> {
        // Fetch plan info to determine if it's a free or paid plan
        let plan_with_version =
            PlanRow::get_with_version(conn, subscription.plan_version_id, subscription.tenant_id)
                .await?;

        let plan = plan_with_version.plan;
        let is_free_plan = plan.plan_type == PlanTypeEnum::Free;

        if is_free_plan {
            // Free plan: transition to Active with no billing
            let new_period_start = subscription
                .current_period_end
                .unwrap_or_else(|| common_domain::clock::now().naive_utc().date());

            let period = calculate_advance_period_range(
                new_period_start,
                subscription.billing_day_anchor as u32,
                true, // Align to billing_day_anchor
                &(subscription.period.clone().into()),
            );

            Ok(NextCycle {
                status: SubscriptionStatusEnum::Active,
                next_cycle_action: Some(CycleActionEnum::RenewSubscription),
                new_period_start,
                new_period_end: Some(period.end),
                should_bill: false, // Free plan - never bill
                pending_checkout: false,
            })
        } else {
            // Free trial on paid plan: determine next action based on activation condition and payment setup
            let is_on_checkout = subscription.activation_condition
                == SubscriptionActivationConditionEnum::OnCheckout;

            if is_on_checkout {
                // OnCheckout: subscription requires successful payment to activate.
                // Always go to TrialExpired, attempt charge if payment method exists.
                // On payment success, on_invoice_payment_settled will transition to Active.
                // On payment failure, stays TrialExpired until customer retries/fixes payment.
                let has_payment_method = if subscription.charge_automatically {
                    let payment_methods = CustomerPaymentMethodRow::list_by_customer_id(
                        conn,
                        &subscription.tenant_id,
                        &subscription.customer_id,
                    )
                    .await?;
                    !payment_methods.is_empty()
                } else {
                    false
                };

                let new_period_start = subscription
                    .current_period_end
                    .unwrap_or_else(|| common_domain::clock::now().naive_utc().date());

                let period = calculate_advance_period_range(
                    new_period_start,
                    subscription.billing_day_anchor as u32,
                    true,
                    &(subscription.period.clone().into()),
                );

                return Ok(NextCycle {
                    status: SubscriptionStatusEnum::TrialExpired,
                    next_cycle_action: None,
                    new_period_start,
                    new_period_end: Some(period.end),
                    should_bill: has_payment_method,
                    pending_checkout: !has_payment_method, // Only pending if no payment method
                });
            }

            // For OnStart/Manual: trust-based billing, subscription already committed.
            // Invoice is created and subscription becomes Active.
            self.renew_subscription(subscription)
        }
    }

    fn renew_subscription(&self, subscription: &SubscriptionRow) -> StoreResult<NextCycle> {
        // Calculate new period
        let new_period_start = subscription
            .current_period_end // cannot be null in this context
            .unwrap_or_else(|| common_domain::clock::now().naive_utc().date());

        let period = calculate_advance_period_range(
            new_period_start,
            subscription.billing_day_anchor as u32,
            false,
            &(subscription.period.clone().into()),
        );

        Ok(NextCycle {
            status: SubscriptionStatusEnum::Active,
            next_cycle_action: Some(CycleActionEnum::RenewSubscription),
            new_period_start,
            new_period_end: Some(period.end),
            should_bill: true,
            pending_checkout: false,
        })
    }

    fn end_subscription(&self, subscription: &SubscriptionRow) -> StoreResult<NextCycle> {
        let new_period_start = subscription
            .current_period_end // cannot be null in this context
            .unwrap_or_else(|| common_domain::clock::now().naive_utc().date());

        Ok(NextCycle {
            status: SubscriptionStatusEnum::Completed,
            next_cycle_action: None,
            new_period_start,
            new_period_end: None,
            should_bill: true,
            pending_checkout: false,
        })
    }
}

impl crate::services::ServicesEdge {
    pub async fn bill_subscription_period_at(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        issued_at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<Option<DetailedInvoice>> {
        self.services
            .process_cycle_transition_at(tenant_id, subscription_id, issued_at, delivery)
            .await
    }
}

struct NextCycle {
    status: SubscriptionStatusEnum,
    next_cycle_action: Option<CycleActionEnum>,
    new_period_start: NaiveDate,
    new_period_end: Option<NaiveDate>,
    should_bill: bool,
    pending_checkout: bool,
}

fn busy_customer_retry_time() -> NaiveDateTime {
    use rand::RngExt;
    let seconds = rand::rng().random_range(BUSY_CUSTOMER_RETRY_SECONDS);
    common_domain::clock::now().naive_utc() + Duration::seconds(seconds)
}

const UNAPPLIED_CHANGE: &str = "a change dated in the current period has not applied yet";

/// The next attempt of a change dated inside the subscription's current period that has not
/// applied yet (`Some(None)`: due now), `None` when there is none.
async fn unapplied_change_of_period(
    conn: &mut PgConn,
    subscription: &SubscriptionRow,
) -> StoreResult<Option<Option<NaiveDateTime>>> {
    let Some(period_end) = subscription.current_period_end else {
        return Ok(None);
    };
    ScheduledEventRow::next_unapplied_change_before(
        conn,
        &subscription.tenant_id,
        subscription.id,
        period_end.and_time(chrono::NaiveTime::MIN),
    )
    .await
    .map_err(Into::into)
}

fn calculate_retry_time(error_count: i32) -> NaiveDateTime {
    // Exponential backoff with reasonable caps for 10 retries
    // 1: 1min, 2: 2min, 3: 5min, 4: 10min, 5: 20min, 6: 30min, 7: 60min, 8: 120min, 9: 240min, 10+: 1 day
    let delay_minutes = match error_count {
        1 => 1,
        2 => 1,
        3 => 3,
        4 => 5,
        5 => 15,
        6 => 30,
        7 => 60, // 1 hour
        8 => 60,
        9 => 180,
        _ => 1440,
    };

    let jitter = rand::random::<u64>() % 60; // up to 1 min
    common_domain::clock::now().naive_utc()
        + Duration::minutes(delay_minutes)
        + Duration::seconds(jitter as i64)
}
