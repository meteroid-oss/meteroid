use crate::StoreResult;
use crate::domain::ScheduledEventTypeEnum;
use crate::domain::customer_payment_methods::ResolvedPaymentMethod;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::domain::pgmq::{PaymentRequestEvent, PgmqMessageNew, PgmqQueue};
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::errors::{StoreError, StoreErrorContainer};
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::Services;
use crate::services::orchestration::payment_transaction_failed::hard_declined_on;
use crate::store::PgConn;
use crate::utils::errors::format_error_chain;
use chrono::{Duration, NaiveDateTime};
use diesel_async::AsyncConnection;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{InvoiceStatusEnum, SubscriptionStatusEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRow;
use diesel_models::scheduled_events::ScheduledEventRow;
use error_stack::Report;
use futures::stream::StreamExt;
use scoped_futures::ScopedFutureExt;

const MAX_PARALLEL_PROCESSING: usize = 4;
const BATCH_SIZE: i64 = (MAX_PARALLEL_PROCESSING * 2) as i64; // Small buffer, small blast radius on crash

impl Services {
    pub async fn cleanup_timeout_scheduled_events(&self) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        ScheduledEventRow::retry_timeout_events(&mut conn).await?;
        Ok(())
    }

    pub async fn get_and_process_due_events(&self) -> StoreResult<usize> {
        let events = {
            let mut conn = self.store.get_conn().await?;
            ScheduledEventRow::find_and_claim_due_events(&mut conn, BATCH_SIZE, None).await?
        };

        let len = events.len();
        if len == 0 {
            return Ok(0);
        }

        // Process each event with bounded parallelism
        let results: Vec<_> = futures::stream::iter(events)
            .map(|event| async move {
                let ids = (event.tenant_id, event.subscription_id, event.id);
                (ids, self.process_scheduled_event(event).await)
            })
            .buffer_unordered(MAX_PARALLEL_PROCESSING)
            .collect()
            .await;

        // Log any unexpected errors (individual event errors are already handled)
        for ((tenant_id, subscription_id, event_id), result) in &results {
            if let Err(e) = result {
                tracing::error!(
                    tenant_id = %tenant_id,
                    subscription_id = %subscription_id,
                    "Unexpected error in scheduled event {event_id}: {e:?}"
                );
            }
        }

        Ok(len)
    }

    /// Process a single event in its own transaction
    #[tracing::instrument(skip_all, fields(
        tenant_id = %event.tenant_id,
        subscription_id = %event.subscription_id,
        event_id = %event.id,
        event_type = ?event.event_type,
    ))]
    pub(super) async fn process_scheduled_event(
        &self,
        event: ScheduledEventRow,
    ) -> StoreResult<()> {
        let event_id = event.id;
        let retries = event.retries;

        let mut conn = self.store.get_conn().await?;
        let event = &event;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                // The handler runs in a savepoint so a failure rolls back everything it wrote
                // before erroring. Without it, the retry marker below commits *alongside* the
                // half-applied work — a balance already debited, component rows already
                // inserted — and the retry re-applies it on top: double charges, duplicate
                // rows, or a consolidation left partially done and never completable.
                let process_result = conn
                    .transaction(async |inner_conn| {
                        services
                            .process_event(inner_conn, event.clone())
                            .await
                            .map_err(Into::<StoreErrorContainer>::into)
                    })
                    .await;

                match process_result {
                    Ok(()) => {
                        ScheduledEventRow::mark_as_completed(conn, &[event_id]).await?;
                    }
                    Err(container) => {
                        // A failure on metering this pass was not served is priced again.
                        services.ensure_usage_served()?;
                        let err: error_stack::Report<StoreError> = container.into();
                        let inner = err.current_context();
                        let error_message = format_error_chain(&err);

                        if super::waits_on_metering(&err) {
                            log::warn!(
                                "Scheduled event {event_id} waits on the metering service: {error_message}"
                            );
                            ScheduledEventRow::defer_event(
                                conn,
                                &event_id,
                                super::metering_retry_time(),
                                &StoreError::MeteringNotReady.to_string(),
                            )
                            .await?;
                        } else if super::violates_invariant(&err) {
                            log::error!(
                                "Scheduled event {event_id} breaks a billing invariant, failed without retry: {error_message}"
                            );
                            ScheduledEventRow::mark_as_failed(conn, &event_id, &error_message)
                                .await?;
                        } else if services.should_retry_event(retries, inner) {
                            let retry_time = calculate_retry_time(retries);
                            log::warn!(
                                "Scheduled event {} failed (attempt {}/5), retrying at {:?}: {}",
                                event_id,
                                retries + 1,
                                retry_time,
                                error_message
                            );
                            ScheduledEventRow::retry_event(
                                conn,
                                &event_id,
                                retry_time,
                                &error_message,
                            )
                            .await?;
                        } else {
                            log::error!(
                                "Scheduled event {} exceeded max retries, marking as failed: {}",
                                event_id,
                                error_message
                            );
                            ScheduledEventRow::mark_as_failed(conn, &event_id, &error_message)
                                .await?;
                        }
                    }
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    /// Process events sequentially on a provided connection (used by period_transitions).
    ///
    /// Each event runs in a savepoint and a failure PROPAGATES instead of writing a retry marker
    /// here: the caller's transaction owns the retry accounting (the subscription's error state),
    /// and it also marks the events completed after this returns. The old behaviour — record a
    /// retry on the shared connection and return Ok — let the caller stamp a *failed* event as
    /// completed, clobbering the retry: the cycle then advanced and billed on half-applied state.
    pub(super) async fn process_event_batch(
        &self,
        conn: &mut PgConn,
        events: Vec<ScheduledEventRow>,
    ) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            for event in events {
                conn.transaction(async |inner_conn| {
                    self.process_event(inner_conn, event)
                        .await
                        .map_err(Into::<StoreErrorContainer>::into)
                })
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            }
            Ok(())
        })
        .await
    }

    // can we batch more ? ex: group by event type before
    /// Process a scheduled event
    async fn process_event(&self, conn: &mut PgConn, event: ScheduledEventRow) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let event: ScheduledEvent = event.try_into().map_err(|_| {
                StoreError::InvalidArgument("Failed to convert ScheduledEventRow".into())
            })?;
            // Customer first: every handler writes the subscription or its invoices, then credit.
            let lock = diesel_models::subscriptions::SubscriptionRow::lock_customer_of(
                conn,
                event.subscription_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            // Process based on event type
            let processed = match event.event_type {
                ScheduledEventTypeEnum::FinalizeInvoice => {
                    self.process_finalize_invoice(conn, &event).await
                }
                ScheduledEventTypeEnum::RetryPayment => {
                    self.process_retry_payment(conn, &event).await
                }
                ScheduledEventTypeEnum::CancelSubscription => {
                    self.process_cancel_subscription(conn, &event).await
                }
                ScheduledEventTypeEnum::ApplyPlanChange
                | ScheduledEventTypeEnum::ApplyAmendment => {
                    self.migrate_due_change(conn, &event).await
                }
                ScheduledEventTypeEnum::PauseSubscription => {
                    self.process_pause_subscription(conn, &event).await
                }
                ScheduledEventTypeEnum::EndTrial => self.process_end_trial(conn, &event).await,
            };
            processed?;
            diesel_models::query::credit_inputs::bump_customer(conn, &lock)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            Ok(())
        })
        .await
    }
    /// A change whose event came due before it was written as facts (pending at the deploy, or
    /// claimed by an old replica): written as its event stored it, then the state derived. One
    /// that cannot be written completes unwritten, its reason recorded on the event.
    async fn migrate_due_change(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        use crate::services::facts::pending::{Writing, write_pending_as_facts_tx};

        let why = match write_pending_as_facts_tx(conn, event, Writing::Due).await? {
            Ok(_) => {
                self.derive_state_tx(
                    conn,
                    &Actor::System,
                    event.tenant_id,
                    event.subscription_id,
                    common_clock::now(),
                )
                .await?;
                return Ok(());
            }
            Err(why) => why,
        };
        log::warn!(
            "Scheduled change {} of subscription {} completed unwritten: {why:?}",
            event.id,
            event.subscription_id,
        );
        ScheduledEventRow::record_error(
            conn,
            &event.tenant_id,
            &event.id,
            &format!("not written as facts: {why:?}"),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }

    /// Determine if event should be retried
    fn should_retry_event(&self, retries: i32, _error: &StoreError) -> bool {
        // TODO retry transient errors, but not configuration or validation errors
        retries < 5
    }

    async fn process_finalize_invoice(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            if let ScheduledEventData::FinalizeInvoice { invoice_id } = event.event_data {
                // Merge this draft with the customer's eligible same-day siblings, else finalize it.
                self.consolidate_and_finalize(conn, event.tenant_id, invoice_id)
                    .await?;
            } else {
                log::error!(
                    "Unexpected event data for type FinalizeInvoice: {:?}, event_id={}",
                    event.event_data,
                    event.id
                );
            }
            Ok(())
        })
        .await
    }

    /// Dunning: re-attempt collection on an invoice whose previous payment failed.
    ///
    /// Enqueues onto the same durable payment path as an automatic first charge rather
    /// than charging inline, so a provider outage retries through pgmq instead of failing
    /// the scheduled event. The next rung of the ladder is scheduled by the failure
    /// handler when this attempt fails in turn.
    async fn process_retry_payment(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        let ScheduledEventData::RetryPayment { invoice_id } = event.event_data else {
            log::error!(
                "Unexpected event data for type RetryPayment: {:?}, event_id={}",
                event.event_data,
                event.id
            );
            return Ok(());
        };

        let invoice = InvoiceRow::find_by_id(conn, event.tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Collected, written off, or cancelled between scheduling and now.
        if invoice.status != InvoiceStatusEnum::Finalized || invoice.amount_due <= 0 {
            return Ok(());
        }

        // Someone got there first (portal payment, manual charge, or a racing retry).
        if PaymentTransactionRow::exists_live_for_invoice(conn, invoice_id, event.tenant_id).await?
        {
            return Ok(());
        }

        let ResolvedPaymentMethod::CustomerPaymentMethod(payment_method_id) = self
            .store
            .resolve_payment_method_for_subscription(event.tenant_id, event.subscription_id)
            .await?
        else {
            // No automatic method (never configured, or the mandate was revoked). Nothing
            // to retry against — the invoice stays in collection for the portal.
            log::info!(
                "No automatic payment method for invoice {}; stopping dunning",
                invoice_id
            );
            return Ok(());
        };

        // A hard decline since this retry was queued: only a new method (or, for an expired
        // card, refreshed card details) revives it.
        let latest_failure =
            PaymentTransactionRow::latest_failed_for_invoice(conn, invoice_id, event.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let method =
            CustomerPaymentMethodRow::get_by_id(conn, &event.tenant_id, &payment_method_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        if latest_failure.is_some_and(|failed| hard_declined_on(&failed, &method)) {
            log::info!(
                "Payment method of invoice {} was hard-declined; skipping retry",
                invoice_id
            );
            return Ok(());
        }

        let msg: PgmqMessageNew =
            PaymentRequestEvent::new(event.tenant_id, invoice_id, payment_method_id).try_into()?;

        // A retried collection is still an auto-charge decision for this invoice.
        if invoice.auto_charge_requested_at.is_none() {
            InvoiceRow::set_auto_charge_requested(
                conn,
                invoice_id,
                event.tenant_id,
                Some(common_clock::now().naive_utc()),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        }

        self.store
            .pgmq_send_batch_tx(conn, PgmqQueue::PaymentRequest, vec![msg])
            .await?;

        Ok(())
    }

    async fn process_cancel_subscription(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            // TODO store the reason & churn data
            if let ScheduledEventData::CancelSubscription {
                reason,
                usage_cutoff,
            } = &event.event_data
            {
                let terminated = self
                    .terminate_subscription(
                        conn,
                        &event.created_by(),
                        event.tenant_id,
                        event.subscription_id,
                        event.scheduled_time.date(),
                        SubscriptionStatusEnum::Cancelled,
                        *usage_cutoff,
                    )
                    .await?;
                // Only emit when the subscription actually transitioned: a scheduled cancel that
                // lands after the subscription already completed/aborted is an idempotent no-op and
                // must not emit a contradictory `subscription.cancelled` (e.g. status=Completed).
                if terminated {
                    self.emit_subscription_cancelled_tx(
                        conn,
                        event.tenant_id,
                        event.subscription_id,
                        reason.clone(),
                    )
                    .await?;
                }
            } else {
                log::error!(
                    "Unexpected event data for type CancelSubscription: {:?}, event_id={}",
                    event.event_data,
                    event.id
                );
            }

            Ok(())
        })
        .await
    }

    async fn process_pause_subscription(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            if let ScheduledEventData::PauseSubscription = &event.event_data {
                let terminated = self
                    .terminate_subscription(
                        conn,
                        &event.created_by(),
                        event.tenant_id,
                        event.subscription_id,
                        event.scheduled_time.date(),
                        SubscriptionStatusEnum::Paused,
                        // Pause settles at the scheduled date's midnight, like a date-effective cancel.
                        None,
                    )
                    .await?;
                if terminated {
                    self.emit_subscription_updated_tx(
                        conn,
                        event.tenant_id,
                        event.subscription_id,
                        SubscriptionUpdateType::Paused,
                    )
                    .await?;
                }
            } else {
                log::error!(
                    "Unexpected event data for type PauseSubscription: {:?}, event_id={}",
                    event.event_data,
                    event.id
                );
            }

            Ok(())
        })
        .await
    }

    /// Process EndTrial event for paid trials.
    /// This transitions the subscription from TrialActive to Active.
    /// Billing continues normally via RenewSubscription - this just handles the status change.
    async fn process_end_trial(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        use common_domain::ids::BaseId;
        use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};

        if let ScheduledEventData::EndTrial = &event.event_data {
            // Get the subscription
            let subscription = SubscriptionRow::get_subscription_by_id(
                conn,
                &event.tenant_id,
                event.subscription_id,
            )
            .await?;

            // Only process if subscription is still in TrialActive status
            if subscription.subscription.status == SubscriptionStatusEnum::TrialActive {
                // Transition to Active - billing continues normally via RenewSubscription
                let patch = SubscriptionCycleRowPatch {
                    id: event.subscription_id,
                    tenant_id: event.tenant_id,
                    status: Some(SubscriptionStatusEnum::Active),
                    cycle_index: None,
                    next_cycle_action: None,
                    current_period_start: None,
                    current_period_end: None,
                    pending_checkout: None,
                    processing_started_at: None,
                    billing_start_date: None,
                    billing_day_anchor: None,
                    terminated_at: None,
                };
                patch.patch(conn).await?;

                log::info!(
                    "Paid trial ended for subscription {}, transitioned to Active",
                    event.subscription_id.as_base62()
                );

                self.emit_subscription_updated_tx(
                    conn,
                    event.tenant_id,
                    event.subscription_id,
                    SubscriptionUpdateType::TrialEnded,
                )
                .await?;
            } else {
                log::warn!(
                    "EndTrial event for subscription {} but status is {:?}, skipping",
                    event.subscription_id.as_base62(),
                    subscription.subscription.status
                );
            }
        } else {
            log::error!(
                "Unexpected event data for type EndTrial: {:?}, event_id={}",
                event.event_data,
                event.id
            );
        }

        Ok(())
    }

    async fn _process_send_payment_reminder(
        &self,
        _conn: &mut PgConn,
        _event: &ScheduledEventRow,
    ) -> StoreResult<()> {
        // TODO
        Ok(())
    }
}

/// backoff for retries
fn calculate_retry_time(retries: i32) -> NaiveDateTime {
    let delay_minutes = match retries {
        1 => 1,
        2 => 5,
        3 => 30,
        _ => 180,
    };

    let jitter = rand::random::<u64>() % 60; // up to 1 min
    common_clock::now().naive_utc()
        + Duration::minutes(delay_minutes)
        + Duration::seconds(jitter as i64)
}
