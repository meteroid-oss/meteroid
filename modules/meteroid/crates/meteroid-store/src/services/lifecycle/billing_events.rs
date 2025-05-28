use crate::StoreResult;
use crate::domain::ScheduledEventTypeEnum;
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use chrono::{Duration, NaiveDateTime, Utc};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::scheduled_events::ScheduledEventRow;
use uuid::Uuid;

const BATCH_SIZE: i64 = 50;

impl Services {
    pub async fn cleanup(&self) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        ScheduledEventRow::retry_timeout_events(&mut conn).await?;
        Ok(())
    }

    pub async fn get_and_process_due_events(&self) -> StoreResult<usize> {
        let events = self
            .store
            .transaction(|tx| {
                async move {
                    // TODO we could process non-terminal events without waiting for cycle
                    let events =
                        ScheduledEventRow::find_due_events_for_update(tx, BATCH_SIZE).await?;

                    if events.is_empty() {
                        return Ok(vec![]);
                    }

                    ScheduledEventRow::mark_as_processing(
                        tx,
                        &events.iter().map(|v| v.id).collect::<Vec<Uuid>>(),
                    )
                    .await?;

                    Ok(events)
                }
                .scope_boxed()
            })
            .await?;

        let len = events.len();

        if len == 0 {
            self.cleanup().await?;
            return Ok(0);
        }

        let mut conn = self.store.get_conn().await?;

        self.process_event_batch(&mut conn, events).await?;

        Ok(len)
    }

    pub(super) async fn process_event_batch(
        &self,
        conn: &mut PgConn,
        event: Vec<ScheduledEventRow>,
    ) -> StoreResult<()> {
        // Process each event TODO batch
        for event in event {
            let event_id = event.id;
            let retries = event.retries;
            match self.process_event(conn, event).await {
                Ok(()) => {
                    log::info!(">>>> Event complete");

                    // Mark as completed
                    ScheduledEventRow::mark_as_completed(conn, &[event_id]) // TODO batch
                        .await?;
                }
                Err(err) => {
                    log::info!(">>>> Event failed");

                    let inner = err.current_context();
                    // Handle error
                    if self.should_retry_event(retries, inner) {
                        // Retry logic
                        let retry_time = calculate_retry_time(retries);
                        ScheduledEventRow::retry_event(
                            conn,
                            &event_id,
                            retry_time,
                            &inner.to_string(),
                        )
                        .await?;
                    } else {
                        // Mark as failed
                        ScheduledEventRow::mark_as_failed(conn, &event_id, &inner.to_string())
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    // can we batch more ? ex: group by event type before
    /// Process a scheduled event
    async fn process_event(&self, conn: &mut PgConn, event: ScheduledEventRow) -> StoreResult<()> {
        let event: ScheduledEvent = event.clone().try_into().map_err(|_| {
            StoreError::InvalidArgument("Failed to convert ScheduledEventRow".into())
        })?;

        // Process based on event type
        let result = match event.event_type {
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
        };

        result
    }
    /// Determine if event should be retried
    fn should_retry_event(&self, retries: i32, _error: &StoreError) -> bool {
        // TODO retry transient errors, but not configuration or validation errors
        retries < 5
    }

    // Event processing implementation methods

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
        if let ScheduledEventData::ApplyPlanChange { .. } = &event.event_data {
            self.terminate_subscription(
                conn,
                event.tenant_id,
                event.subscription_id,
                event.scheduled_time.date(),
                SubscriptionStatusEnum::Superseded,
            )
            .await?;

            // TODO we need more things in that event, to be able to initiate the subscription
            todo!();
        } else {
            log::error!(
                "Unexpected event data for type CancelSubscription: {:?}, event_id={}",
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
