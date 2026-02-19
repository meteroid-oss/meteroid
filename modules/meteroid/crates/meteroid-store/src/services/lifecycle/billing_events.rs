use crate::StoreResult;
use crate::domain::ScheduledEventTypeEnum;
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::store::PgConn;
use crate::utils::errors::format_error_chain;
use chrono::{Duration, NaiveDateTime, Utc};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{SubscriptionEventType, SubscriptionStatusEnum};
use diesel_models::scheduled_events::ScheduledEventRow;
use futures::stream::StreamExt;

const MAX_PARALLEL_PROCESSING: usize = 4;
const BATCH_SIZE: i64 = (MAX_PARALLEL_PROCESSING * 2) as i64; // Small buffer, small blast radius on crash

impl Services {
    pub async fn cleanup_timeout_scheduled_events(&self) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        ScheduledEventRow::retry_timeout_events(&mut conn).await?;
        Ok(())
    }

    pub async fn get_and_process_due_events(&self) -> StoreResult<usize> {
        let mut conn = self.store.get_conn().await?;
        let events = ScheduledEventRow::find_and_claim_due_events(&mut conn, BATCH_SIZE).await?;

        let len = events.len();
        if len == 0 {
            return Ok(0);
        }

        // Process each event with bounded parallelism
        let results: Vec<_> = futures::stream::iter(events)
            .map(|event| self.process_single_event(event))
            .buffer_unordered(MAX_PARALLEL_PROCESSING)
            .collect()
            .await;

        // Log any unexpected errors (individual event errors are already handled)
        for result in &results {
            if let Err(e) = result {
                log::error!("Unexpected error in event processing: {:?}", e);
            }
        }

        Ok(len)
    }

    /// Process a single event in its own transaction
    async fn process_single_event(&self, event: ScheduledEventRow) -> StoreResult<()> {
        let event_id = event.id;
        let retries = event.retries;

        self.store
            .transaction(|conn| {
                async move {
                    match self.process_event(conn, event).await {
                        Ok(()) => {
                            ScheduledEventRow::mark_as_completed(conn, &[event_id]).await?;
                        }
                        Err(err) => {
                            let inner = err.current_context();
                            let error_message = format_error_chain(&err);

                            if self.should_retry_event(retries, inner) {
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

    /// Process events sequentially on a provided connection (used by period_transitions)
    pub(super) async fn process_event_batch(
        &self,
        conn: &mut PgConn,
        events: Vec<ScheduledEventRow>,
    ) -> StoreResult<()> {
        for event in events {
            let event_id = event.id;
            let retries = event.retries;
            match self.process_event(conn, event).await {
                Ok(()) => {
                    ScheduledEventRow::mark_as_completed(conn, &[event_id]).await?;
                }
                Err(err) => {
                    let inner = err.current_context();
                    let error_message = format_error_chain(&err);

                    if self.should_retry_event(retries, inner) {
                        let retry_time = calculate_retry_time(retries);
                        log::warn!(
                            "Scheduled event {} failed (attempt {}/5), retrying at {:?}: {}",
                            event_id,
                            retries + 1,
                            retry_time,
                            error_message
                        );
                        ScheduledEventRow::retry_event(conn, &event_id, retry_time, &error_message)
                            .await?;
                    } else {
                        log::error!(
                            "Scheduled event {} exceeded max retries, marking as failed: {}",
                            event_id,
                            error_message
                        );
                        ScheduledEventRow::mark_as_failed(conn, &event_id, &error_message).await?;
                    }
                }
            }
        }
        Ok(())
    }

    // can we batch more ? ex: group by event type before
    /// Process a scheduled event
    async fn process_event(&self, conn: &mut PgConn, event: ScheduledEventRow) -> StoreResult<()> {
        let event: ScheduledEvent = event.try_into().map_err(|_| {
            StoreError::InvalidArgument("Failed to convert ScheduledEventRow".into())
        })?;

        // Process based on event type
        match event.event_type {
            ScheduledEventTypeEnum::FinalizeInvoice => {
                self.process_finalize_invoice(conn, &event).await
            }
            ScheduledEventTypeEnum::RetryPayment => self.process_retry_payment(conn, &event).await,
            ScheduledEventTypeEnum::CancelSubscription => {
                self.process_cancel_subscription(conn, &event).await
            }
            ScheduledEventTypeEnum::ApplyPlanChange => {
                self.process_apply_plan_change(conn, &event).await
            }
            ScheduledEventTypeEnum::PauseSubscription => {
                self.process_pause_subscription(conn, &event).await
            }
            ScheduledEventTypeEnum::EndTrial => self.process_end_trial(conn, &event).await,
        }
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
        if let ScheduledEventData::FinalizeInvoice { invoice_id } = event.event_data {
            self.finalize_invoice_tx(conn, invoice_id, event.tenant_id, true, &None)
                .await?;
        } else {
            log::error!(
                "Unexpected event data for type FinalizeInvoice: {:?}, event_id={}",
                event.event_data,
                event.id
            );
        }
        Ok(())
    }

    async fn process_retry_payment(
        &self,
        _conn: &mut PgConn,
        _event: &ScheduledEvent,
    ) -> StoreResult<()> {
        // TODO
        Ok(())
    }

    async fn process_cancel_subscription(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        // TODO store the reason & churn data
        if let ScheduledEventData::CancelSubscription { .. } = &event.event_data {
            self.terminate_subscription(
                conn,
                event.tenant_id,
                event.subscription_id,
                event.scheduled_time.date(),
                SubscriptionStatusEnum::Cancelled,
            )
            .await?;
        } else {
            log::error!(
                "Unexpected event data for type CancelSubscription: {:?}, event_id={}",
                event.event_data,
                event.id
            );
        }

        Ok(())
    }

    async fn process_apply_plan_change(
        &self,
        conn: &mut PgConn,
        event: &ScheduledEvent,
    ) -> StoreResult<()> {
        use crate::domain::scheduled_events::ComponentMapping;
        use crate::domain::subscription_components::{SubscriptionComponentNew, SubscriptionComponentNewInternal};
        use crate::services::subscriptions::utils::calculate_mrr;
        use diesel_models::subscription_components::{SubscriptionComponentRow, SubscriptionComponentRowNew};
        use diesel_models::subscriptions::SubscriptionRow;

        if let ScheduledEventData::ApplyPlanChange {
            new_plan_version_id,
            ref component_mappings,
        } = event.event_data
        {
            // 1. Update subscription's plan_version_id
            SubscriptionRow::update_plan_version(
                conn,
                &event.subscription_id,
                &event.tenant_id,
                new_plan_version_id,
            )
            .await
            .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            // 2. Process component mappings
            let mut components_to_delete = Vec::new();
            let mut components_to_insert: Vec<SubscriptionComponentRowNew> = Vec::new();
            let apply_date = event.scheduled_time.date();

            for mapping in component_mappings {
                match mapping {
                    ComponentMapping::Matched {
                        current_component_id,
                        target_component_id,
                        price_id,
                        name,
                        fee,
                        period,
                        ..
                    } => {
                        let fee_json: serde_json::Value = fee.clone().try_into().map_err(|_| {
                            StoreError::InvalidArgument(
                                "Failed to serialize fee for plan change".to_string(),
                            )
                        })?;

                        SubscriptionComponentRow::update_for_plan_change(
                            conn,
                            *current_component_id,
                            *target_component_id,
                            Some(*price_id),
                            name.clone(),
                            fee_json,
                            (*period).into(),
                        )
                        .await
                        .map_err(Into::<error_stack::Report<StoreError>>::into)?;
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
                        components_to_delete.push(*current_component_id);
                    }
                }
            }

            // Delete removed components
            SubscriptionComponentRow::delete_by_ids(conn, &components_to_delete)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;

            // Insert new components
            if !components_to_insert.is_empty() {
                let refs: Vec<&SubscriptionComponentRowNew> = components_to_insert.iter().collect();
                SubscriptionComponentRow::insert_subscription_component_batch(conn, refs)
                    .await
                    .map_err(Into::<error_stack::Report<StoreError>>::into)?;
            }

            // 3. Insert Switch subscription event
            let sub_event = diesel_models::subscription_events::SubscriptionEventRow {
                id: uuid::Uuid::now_v7(),
                subscription_id: event.subscription_id,
                event_type: SubscriptionEventType::Switch,
                details: Some(serde_json::json!({
                    "new_plan_version_id": new_plan_version_id.to_string(),
                })),
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
            let sub_details = self
                .store
                .get_subscription_details_with_conn(
                    conn,
                    event.tenant_id,
                    event.subscription_id,
                )
                .await?;

            let precision = crate::constants::Currencies::resolve_currency_precision(
                &sub_details.subscription.currency,
            )
            .unwrap_or(2);

            let new_mrr: i64 = sub_details
                .price_components
                .iter()
                .map(|c| calculate_mrr(&c.fee, &c.period, precision))
                .sum();

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
            }

            log::info!(
                "Applied plan change for subscription {}: plan_version={}, matched={}, added={}, removed={}, mrr_delta={}",
                event.subscription_id,
                new_plan_version_id,
                component_mappings.iter().filter(|m| matches!(m, ComponentMapping::Matched { .. })).count(),
                component_mappings.iter().filter(|m| matches!(m, ComponentMapping::Added { .. })).count(),
                component_mappings.iter().filter(|m| matches!(m, ComponentMapping::Removed { .. })).count(),
                mrr_delta,
            );
        } else {
            log::error!(
                "Unexpected event data for type ApplyPlanChange: {:?}, event_id={}",
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
        if let ScheduledEventData::PauseSubscription = &event.event_data {
            self.terminate_subscription(
                conn,
                event.tenant_id,
                event.subscription_id,
                event.scheduled_time.date(),
                SubscriptionStatusEnum::Paused,
            )
            .await?;
        } else {
            log::error!(
                "Unexpected event data for type PauseSubscription: {:?}, event_id={}",
                event.event_data,
                event.id
            );
        }

        Ok(())
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
                };
                patch.patch(conn).await?;

                log::info!(
                    "Paid trial ended for subscription {}, transitioned to Active",
                    event.subscription_id.as_base62()
                );
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
