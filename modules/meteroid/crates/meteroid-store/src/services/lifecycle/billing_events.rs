use crate::StoreResult;
use crate::domain::ScheduledEventTypeEnum;
use crate::domain::customer_payment_methods::ResolvedPaymentMethod;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::domain::pgmq::{PaymentRequestEvent, PgmqMessageNew, PgmqQueue};
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::domain::slot_transactions::SlotTransactionNewInternal;
use crate::errors::{StoreError, StoreErrorContainer};
use crate::repositories::SubscriptionInterface;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::Services;
use crate::services::orchestration::payment_transaction_failed::hard_declined_on;
use crate::store::PgConn;
use crate::utils::errors::format_error_chain;
use chrono::{Duration, NaiveDateTime, Utc};
use common_domain::ids::BaseId;
use diesel_async::AsyncConnection;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{InvoiceStatusEnum, SubscriptionEventType, SubscriptionStatusEnum};
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
            ScheduledEventRow::find_and_claim_due_events(&mut conn, BATCH_SIZE).await?
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
    async fn process_scheduled_event(&self, event: ScheduledEventRow) -> StoreResult<()> {
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
                ScheduledEventTypeEnum::ApplyPlanChange => {
                    self.process_apply_plan_change(conn, &event, Applying::Lifecycle)
                        .await
                }
                ScheduledEventTypeEnum::PauseSubscription => {
                    self.process_pause_subscription(conn, &event).await
                }
                ScheduledEventTypeEnum::EndTrial => self.process_end_trial(conn, &event).await,
                ScheduledEventTypeEnum::ApplyAmendment => {
                    self.process_apply_amendment(conn, &event, Applying::Lifecycle)
                        .await
                }
            };
            processed?;
            diesel_models::query::credit_inputs::bump_customer(conn, &lock)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            Ok(())
        })
        .await
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
                Some(Utc::now().naive_utc()),
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

    pub(in crate::services) async fn process_apply_plan_change(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
        applying: Applying,
    ) -> StoreResult<()> {
        use crate::domain::scheduled_events::ComponentMapping;
        use crate::domain::subscription_components::{
            SubscriptionComponentNew, SubscriptionComponentNewInternal,
        };
        use crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots;
        use diesel_models::plan_version_add_ons::PlanVersionAddOnRow;
        use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
        use diesel_models::subscription_components::{
            SubscriptionComponentRow, SubscriptionComponentRowNew,
        };
        use diesel_models::subscriptions::SubscriptionRow;
        let level = applying.log_level();

        if let ScheduledEventData::ApplyPlanChange {
            source_plan_version_id,
            new_plan_version_id,
            ref component_mappings,
        } = event.event_data
        {
            // Lock subscription to serialize with immediate plan changes.
            SubscriptionRow::lock_subscription_for_update(conn, event.subscription_id)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            let sub = SubscriptionRow::get_subscription_by_id(
                conn,
                &event.tenant_id,
                event.subscription_id,
            )
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            // Precondition: if another plan change already landed, this event is stale.
            if let Some(expected) = source_plan_version_id
                && sub.subscription.plan_version_id != expected
            {
                log::log!(
                    target: applying.target(),
                    level,
                    "Plan change event {} superseded for subscription {}: expected plan_version {:?}, found {:?}",
                    event.id,
                    event.subscription_id,
                    expected,
                    sub.subscription.plan_version_id,
                );
                return Ok(());
            }

            match sub.subscription.status {
                SubscriptionStatusEnum::Active | SubscriptionStatusEnum::TrialActive => {}
                other => {
                    log::log!(
                        target: applying.target(),
                        applying.warn_level(),
                        "Skipping plan change for subscription {} — status is {:?}, event_id={}",
                        event.subscription_id,
                        other,
                        event.id,
                    );
                    return Err(StoreError::InvalidArgument(format!(
                        "Cannot apply plan change: subscription is in {:?} status",
                        other
                    ))
                    .into());
                }
            }

            // 1. Update subscription's plan_version_id (and billing period if changed)
            let new_period =
                ComponentMapping::derive_billing_period(component_mappings).map(|p| p.into());

            SubscriptionRow::update_plan_version(
                conn,
                &event.subscription_id,
                &event.tenant_id,
                new_plan_version_id,
                new_period,
            )
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            // A preview of the upcoming invoice writes no signal.
            if matches!(applying, Applying::Lifecycle) {
                self.store
                    .hooks
                    .signals
                    .subscription_plan_changed(
                        conn,
                        event.tenant_id,
                        sub.subscription.customer_id,
                        event.subscription_id,
                        Utc::now().naive_utc(),
                    )
                    .await?;
            }
            // The new plan's minimum commitment starts with the period it is scheduled for; the
            // period ending here is trued up under the terms it ran on, recorded on the switch.
            let outgoing_minimum = crate::repositories::subscriptions::load_subscription_minimum(
                conn,
                event.subscription_id,
            )
            .await?;
            let outgoing_terms = crate::services::invoice_lines::minimum::outgoing_terms_record(
                outgoing_minimum.as_ref(),
                sub.subscription.period.clone().into(),
            );
            crate::services::subscriptions::plan_change::refresh_subscription_minimum_on_plan_change(
                conn,
                event.subscription_id,
                new_plan_version_id,
            )
            .await?;

            // 2. Process component mappings using temporal rotation
            let mut components_to_close = Vec::new();
            let mut components_to_insert: Vec<SubscriptionComponentRowNew> = Vec::new();
            let apply_date = event.scheduled_time.date();

            for mapping in component_mappings {
                match mapping {
                    ComponentMapping::Matched {
                        current_component_id,
                        target_component_id,
                        product_id,
                        price_id,
                        name,
                        fee,
                        period,
                    } => {
                        // Close the old component
                        components_to_close.push(*current_component_id);

                        // Insert a new component with effective_from = apply_date
                        let row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
                            subscription_id: event.subscription_id,
                            internal: SubscriptionComponentNewInternal {
                                price_component_id: Some(*target_component_id),
                                product_id: Some(*product_id),
                                name: name.clone(),
                                period: *period,
                                fee: fee.clone(),
                                is_override: false,
                                price_id: Some(*price_id),
                                effective_from: apply_date,
                            },
                        }
                        .try_into()
                        .map_err(|_| {
                            StoreError::InvalidArgument(
                                "Failed to convert matched component for plan change".to_string(),
                            )
                        })?;

                        components_to_insert.push(row_new);
                    }
                    ComponentMapping::Added {
                        target_component_id,
                        product_id,
                        price_id,
                        name,
                        fee,
                        period,
                    } => {
                        let row_new: SubscriptionComponentRowNew = SubscriptionComponentNew {
                            subscription_id: event.subscription_id,
                            internal: SubscriptionComponentNewInternal {
                                price_component_id: Some(*target_component_id),
                                product_id: *product_id,
                                name: name.clone(),
                                period: *period,
                                fee: fee.clone(),
                                is_override: false,
                                price_id: *price_id,
                                effective_from: apply_date,
                            },
                        }
                        .try_into()
                        .map_err(|_| {
                            StoreError::InvalidArgument(
                                "Failed to convert new component for plan change".to_string(),
                            )
                        })?;

                        components_to_insert.push(row_new);
                    }
                    ComponentMapping::Removed {
                        current_component_id,
                    } => {
                        components_to_close.push(*current_component_id);
                    }
                }
            }

            // Close old components (set effective_to)
            SubscriptionComponentRow::close_components(conn, &components_to_close, apply_date)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            // Insert new components
            if !components_to_insert.is_empty() {
                let refs: Vec<&SubscriptionComponentRowNew> = components_to_insert.iter().collect();
                SubscriptionComponentRow::insert_subscription_component_batch(conn, refs)
                    .await
                    .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            }

            // Seed slot transactions for newly added Slot components
            for mapping in component_mappings {
                if let ComponentMapping::Added { fee, .. } = mapping
                    && let Some(tx) = SlotTransactionNewInternal::from_fee(fee, apply_date)
                {
                    tx.into_row(event.subscription_id)
                        .insert(conn)
                        .await
                        .map_err(Into::<error_stack::Report<StoreError>>::into)?;
                }
            }

            // Remove subscription add-ons incompatible with the new plan version
            let compatible_addon_ids: Vec<common_domain::ids::AddOnId> =
                PlanVersionAddOnRow::list_by_plan_version_id(
                    conn,
                    new_plan_version_id,
                    event.tenant_id,
                )
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?
                .into_iter()
                .map(|pva| pva.add_on_id)
                .collect();

            let removed_addons = SubscriptionAddOnRow::delete_incompatible(
                conn,
                &event.subscription_id,
                &compatible_addon_ids,
            )
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            if !removed_addons.is_empty() {
                log::log!(
                    target: applying.target(),
                    level,
                    "Removed {} incompatible add-on(s) during scheduled plan change for subscription {}: {:?}",
                    removed_addons.len(),
                    event.subscription_id,
                    removed_addons
                        .iter()
                        .map(|a| a.add_on_id)
                        .collect::<Vec<_>>(),
                );
            }

            // 3. Insert Switch subscription event
            let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
                id: uuid::Uuid::now_v7(),
                subscription_id: event.subscription_id,
                event_type: SubscriptionEventType::Switch,
                details: Some({
                    let mut details = serde_json::json!({
                        "new_plan_version_id": new_plan_version_id.to_string(),
                        "old_plan_version_id": sub.subscription.plan_version_id.to_string(),
                    });
                    for (key, value) in outgoing_terms {
                        details[key] = value;
                    }
                    details
                }),
                created_at: chrono::Utc::now().naive_utc(),
                mrr_delta: None, // MRR delta computed below
                bi_mrr_movement_log_id: None,
                applies_to: apply_date,
            };
            sub_event
                .insert(conn)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            // 4. Recalculate MRR
            let mut sub_details = self
                .store
                .get_subscription_details_with_conn(conn, event.tenant_id, event.subscription_id)
                .await?;

            let precision = crate::constants::Currencies::resolve_currency_precision(
                &sub_details.subscription.currency,
            )
            .unwrap_or(2);

            let component_lines = calculate_components_mrr_with_slots(
                conn,
                event.tenant_id,
                event.subscription_id,
                &sub_details.price_components,
                precision,
            )
            .await?;

            let new_mrr = self
                .restate_subscription_mrr(
                    conn,
                    &mut sub_details,
                    component_lines,
                    precision,
                    apply_date,
                )
                .await?;
            let old_mrr = sub_details.subscription.mrr_cents as i64;
            let mrr_delta = new_mrr - old_mrr;

            if mrr_delta != 0 {
                SubscriptionRow::update_subscription_mrr_delta(
                    conn,
                    event.subscription_id,
                    mrr_delta,
                )
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
                // The scheduled change takes effect now (apply_date), so book the
                // Expansion/Contraction here — at application time, not when it was scheduled.
                self.create_movement_mrr_log(
                    conn,
                    event.tenant_id,
                    event.subscription_id,
                    apply_date,
                    mrr_delta,
                )
                .await?;
            }

            // Trial → Active transition (paid trial with scheduled plan change)
            if sub.subscription.status == SubscriptionStatusEnum::TrialActive {
                use diesel_models::subscriptions::SubscriptionCycleRowPatch;

                // Cancel the EndTrial scheduled event
                ScheduledEventRow::cancel_pending_subscription_events(
                    conn,
                    event.subscription_id,
                    &event.tenant_id,
                    "Plan change during paid trial (scheduled)",
                )
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

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

                log::log!(
                    target: applying.target(),
                    level,
                    "Paid trial ended via scheduled plan change for subscription {}",
                    event.subscription_id,
                );
            }

            log::log!(
                target: applying.target(),
                level,
                "Applied plan change for subscription {}: plan_version={}, matched={}, added={}, removed={}, mrr_delta={}",
                event.subscription_id,
                new_plan_version_id,
                component_mappings
                    .iter()
                    .filter(|m| matches!(m, ComponentMapping::Matched { .. }))
                    .count(),
                component_mappings
                    .iter()
                    .filter(|m| matches!(m, ComponentMapping::Added { .. }))
                    .count(),
                component_mappings
                    .iter()
                    .filter(|m| matches!(m, ComponentMapping::Removed { .. }))
                    .count(),
                mrr_delta,
            );

            self.emit_subscription_updated_tx(
                conn,
                event.tenant_id,
                event.subscription_id,
                SubscriptionUpdateType::PlanChanged,
            )
            .await?;

            // Scheduled change now applied — the request-time row was plan_change_scheduled.
            let activity = Activity::new(
                ActivityType::SubscriptionPlanChanged,
                EntityType::Subscription,
                event.subscription_id.as_uuid(),
            )
            .with_metadata(serde_json::json!({
                "new_plan_version_id": new_plan_version_id.to_string(),
                "change_date": apply_date.to_string(),
            }))
            .agg_customer(sub.subscription.customer_id);
            self.store
                .record_tx(
                    conn,
                    event.tenant_id,
                    &Actor::System,
                    AuditInput::Activity(activity),
                )
                .await?;
        } else {
            log::error!(
                "Unexpected event data for type ApplyPlanChange: {:?}, event_id={}",
                event.event_data,
                event.id
            );
        }
        Ok(())
    }

    /// Apply a manual/sales-led amendment scheduled for end-of-period.
    /// Mirrors `process_apply_plan_change` but keeps the plan version and period:
    /// it only rotates the carried component/add-on deltas at the scheduled date.
    pub(in crate::services) async fn process_apply_amendment(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
        applying: Applying,
    ) -> StoreResult<()> {
        use crate::domain::subscription_components::{
            SubscriptionComponentNew, SubscriptionComponentNewInternal,
        };
        use crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots;
        use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
        use diesel_models::subscription_components::{
            SubscriptionComponentRow, SubscriptionComponentRowNew,
        };
        use diesel_models::subscriptions::SubscriptionRow;
        let level = applying.log_level();

        if let ScheduledEventData::ApplyAmendment {
            component_close,
            component_insert,
            addon_close,
            addon_insert,
        } = &event.event_data
        {
            SubscriptionRow::lock_subscription_for_update(conn, event.subscription_id)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            let sub = SubscriptionRow::get_subscription_by_id(
                conn,
                &event.tenant_id,
                event.subscription_id,
            )
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            match sub.subscription.status {
                SubscriptionStatusEnum::Active | SubscriptionStatusEnum::TrialActive => {}
                other => {
                    log::log!(
                        target: applying.target(),
                        applying.warn_level(),
                        "Skipping amendment for subscription {} — status is {:?}, event_id={}",
                        event.subscription_id,
                        other,
                        event.id,
                    );
                    return Err(StoreError::InvalidArgument(format!(
                        "Cannot apply amendment: subscription is in {:?} status",
                        other
                    ))
                    .into());
                }
            }

            let apply_date = event.scheduled_time.date();

            // Close removed/edited components and add-ons.
            SubscriptionComponentRow::close_components(conn, component_close, apply_date)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            SubscriptionAddOnRow::close_addons(conn, addon_close, apply_date)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            // Insert new components.
            let component_rows: Vec<SubscriptionComponentRowNew> = component_insert
                .iter()
                .map(|c| {
                    let mut row: SubscriptionComponentRowNew = SubscriptionComponentNew {
                        subscription_id: event.subscription_id,
                        internal: SubscriptionComponentNewInternal {
                            price_component_id: c.price_component_id,
                            product_id: c.product_id,
                            name: c.name.clone(),
                            period: c.period,
                            fee: c.fee.clone(),
                            is_override: c.is_override,
                            price_id: c.price_id,
                            effective_from: apply_date,
                        },
                    }
                    .try_into()?;
                    // Preserve the overridden component's lineage so amendment credits
                    // stay matched to the originally-billed invoice line across overrides.
                    row.lineage_id = c.lineage_id;
                    // Mark as amendment-added so a one-time fee bills on its effective period.
                    row.added_by_amendment = true;
                    Ok::<_, error_stack::Report<StoreError>>(row)
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_: error_stack::Report<StoreError>| {
                    StoreError::InvalidArgument("Failed to convert amended component".to_string())
                })?;

            if !component_rows.is_empty() {
                let refs: Vec<&SubscriptionComponentRowNew> = component_rows.iter().collect();
                SubscriptionComponentRow::insert_subscription_component_batch(conn, refs)
                    .await
                    .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            }

            // Insert new add-ons.
            let addon_rows: Vec<SubscriptionAddOnRowNew> = addon_insert
                .iter()
                .map(|a| {
                    let mut row: SubscriptionAddOnRowNew =
                        crate::domain::subscription_add_ons::SubscriptionAddOnNew {
                            subscription_id: event.subscription_id,
                            internal:
                                crate::domain::subscription_add_ons::SubscriptionAddOnNewInternal {
                                    add_on_id: a.add_on_id,
                                    name: a.name.clone(),
                                    period: a.period,
                                    fee: a.fee.clone(),
                                    product_id: a.product_id,
                                    price_id: a.price_id,
                                    quantity: a.quantity,
                                    effective_from: apply_date,
                                },
                        }
                        .try_into()?;
                    row.lineage_id = a.lineage_id;
                    row.added_by_amendment = true;
                    Ok::<_, error_stack::Report<StoreError>>(row)
                })
                .collect::<Result<Vec<_>, error_stack::Report<StoreError>>>()?;

            if !addon_rows.is_empty() {
                let refs: Vec<&SubscriptionAddOnRowNew> = addon_rows.iter().collect();
                SubscriptionAddOnRow::insert_batch(conn, refs)
                    .await
                    .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            }

            // Seed slot transactions for newly added Slot components/add-ons.
            for c in component_insert {
                if let Some(tx) = SlotTransactionNewInternal::from_fee(&c.fee, apply_date) {
                    tx.into_row(event.subscription_id)
                        .insert(conn)
                        .await
                        .map_err(Into::<error_stack::Report<StoreError>>::into)?;
                }
            }
            for a in addon_insert {
                if let Some(tx) = SlotTransactionNewInternal::from_fee(&a.fee, apply_date) {
                    tx.into_row(event.subscription_id)
                        .insert(conn)
                        .await
                        .map_err(Into::<error_stack::Report<StoreError>>::into)?;
                }
            }

            let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
                id: uuid::Uuid::now_v7(),
                subscription_id: event.subscription_id,
                event_type: SubscriptionEventType::Switch,
                details: Some(serde_json::json!({ "kind": "amendment", "mode": "end_of_period" })),
                created_at: chrono::Utc::now().naive_utc(),
                mrr_delta: None,
                bi_mrr_movement_log_id: None,
                applies_to: apply_date,
            };
            sub_event
                .insert(conn)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            // Recalculate MRR.
            let mut sub_details = self
                .store
                .get_subscription_details_with_conn(conn, event.tenant_id, event.subscription_id)
                .await?;

            let precision = crate::constants::Currencies::resolve_currency_precision(
                &sub_details.subscription.currency,
            )
            .unwrap_or(2);

            let component_lines = calculate_components_mrr_with_slots(
                conn,
                event.tenant_id,
                event.subscription_id,
                &sub_details.price_components,
                precision,
            )
            .await?;

            let new_mrr = self
                .restate_subscription_mrr(
                    conn,
                    &mut sub_details,
                    component_lines,
                    precision,
                    apply_date,
                )
                .await?;
            let old_mrr = sub_details.subscription.mrr_cents as i64;
            let mrr_delta = new_mrr - old_mrr;

            if mrr_delta != 0 {
                SubscriptionRow::update_subscription_mrr_delta(
                    conn,
                    event.subscription_id,
                    mrr_delta,
                )
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
                // The scheduled change takes effect now (apply_date), so book the
                // Expansion/Contraction here — at application time, not when it was scheduled.
                self.create_movement_mrr_log(
                    conn,
                    event.tenant_id,
                    event.subscription_id,
                    apply_date,
                    mrr_delta,
                )
                .await?;
            }

            log::log!(
                target: applying.target(),
                level,
                "Applied amendment for subscription {}: closed_components={}, added_components={}, closed_addons={}, added_addons={}, mrr_delta={}",
                event.subscription_id,
                component_close.len(),
                component_insert.len(),
                addon_close.len(),
                addon_insert.len(),
                mrr_delta,
            );

            self.emit_subscription_updated_tx(
                conn,
                event.tenant_id,
                event.subscription_id,
                SubscriptionUpdateType::Amended,
            )
            .await?;

            // Scheduled amendment now applied — the request-time row was amendment_scheduled.
            let activity = Activity::new(
                ActivityType::SubscriptionAmended,
                EntityType::Subscription,
                event.subscription_id.as_uuid(),
            )
            .with_metadata(serde_json::json!({
                "effective_date": apply_date.to_string(),
            }))
            .agg_customer(sub.subscription.customer_id);
            self.store
                .record_tx(
                    conn,
                    event.tenant_id,
                    &Actor::System,
                    AuditInput::Activity(activity),
                )
                .await?;
        } else {
            log::error!(
                "Unexpected event data for type ApplyAmendment: {:?}, event_id={}",
                event.event_data,
                event.id
            );
        }
        Ok(())
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
    Utc::now().naive_utc() + Duration::minutes(delay_minutes) + Duration::seconds(jitter as i64)
}

/// Who applies a scheduled change: the lifecycle, or a preview that rolls it back. A preview logs
/// at debug under its own target, so nothing it does reads as applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::services) enum Applying {
    Lifecycle,
    Preview,
}

impl Applying {
    fn log_level(self) -> log::Level {
        match self {
            Self::Lifecycle => log::Level::Info,
            Self::Preview => log::Level::Debug,
        }
    }

    fn warn_level(self) -> log::Level {
        match self {
            Self::Lifecycle => log::Level::Warn,
            Self::Preview => log::Level::Debug,
        }
    }

    fn target(self) -> &'static str {
        match self {
            Self::Lifecycle => module_path!(),
            Self::Preview => "meteroid_store::preview",
        }
    }
}
