use crate::StoreResult;
use crate::errors::{StoreError, StoreErrorContainer};
use crate::services::{InvoiceBillingMode, Services};
use crate::store::PgConn;
use crate::utils::errors::format_error_chain;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Days, Duration, NaiveDate, NaiveDateTime, Utc};
use common_domain::ids::SubscriptionId;
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{
    CycleActionEnum, PlanTypeEnum, ScheduledEventTypeEnum, SubscriptionActivationConditionEnum,
    SubscriptionStatusEnum,
};
use diesel_models::plans::PlanRow;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscriptions::{
    SubscriptionCycleErrorRowPatch, SubscriptionCycleRowPatch, SubscriptionRow,
};
use error_stack::Report;
use futures::stream::StreamExt;

const BATCH_SIZE: i64 = 8;
const MAX_CYCLE_RETRIES: i32 = 10;
const MAX_PARALLEL_PROCESSING: usize = 4;

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

        // Phase 2: Process each subscription with bounded parallelism.
        let results: Vec<_> = futures::stream::iter(claimed_ids)
            .map(|id| self.process_single_subscription(id))
            .buffer_unordered(MAX_PARALLEL_PROCESSING)
            .collect()
            .await;

        // Log any errors (they're already handled per-subscription)
        for result in &results {
            if let Err(e) = result {
                log::error!("Unexpected error in subscription processing: {:?}", e);
            }
        }

        Ok(CycleTransitionResult {
            processed: claimed_count,
            has_more: claimed_count == BATCH_SIZE as usize,
        })
    }

    /// Lock and processes a single subscription in its own transaction.
    /// Uses a savepoint for the actual processing to record the error on failure:
    async fn process_single_subscription(
        &self,
        subscription_id: SubscriptionId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    // Lock and re-validate the subscription
                    let Some(subscription) =
                        SubscriptionRow::get_and_lock_for_processing(conn, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                    else {
                        log::debug!(
                            "Subscription {} no longer due for processing, skipping",
                            subscription_id
                        );
                        return Ok(());
                    };

                    let tenant_id = subscription.tenant_id;
                    let subscription_id = subscription.id;
                    let error_count = subscription.error_count;


                    // Try processing in a savepoint (nested transaction)
                    let process_result = conn
                        .transaction(|inner_conn| {
                            async move {
                                self.process_cycle_transition(inner_conn, &subscription)
                                    .await
                                    .map_err(Into::<StoreErrorContainer>::into)
                            }
                            .scope_boxed()
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
                        Err(err) => {
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
                        }
                    }

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// Executes the current scheduled action for a subscription.
    /// (activate, renew, end trial, end subscription, etc.)
    async fn process_cycle_transition(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
    ) -> StoreResult<()> {
        // filter terminal states just in case
        let next_action = if let Some(action) = &subscription.next_cycle_action {
            action
        } else {
            log::warn!(
                "No next cycle action for subscription {}. Skipping processing.",
                subscription.id
            );
            return Ok(());
        };

        let mut next_cycle = match next_action {
            CycleActionEnum::ActivateSubscription => self.activate_subscription(subscription)?,
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
                    ScheduledEventTypeEnum::PauseSubscription,
                ],
                next_cycle.new_period_start,
            )
            .await?;

            if let Some(event) = next_event {
                log::info!(
                    "Found next event for subscription {}: {:?}",
                    subscription.id,
                    event
                );
                ScheduledEventRow::mark_as_processing(conn, &[event.id]).await?;
                self.process_event_batch(conn, vec![event]).await?;
                return Ok(());
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
                    None => Some(0), // Should not happen, but handle gracefully
                }
            }
            CycleActionEnum::EndTrial => {
                // Trial end starts billing but doesn't increment cycle
                // (we're still in cycle 0, just now with billing)
                subscription.cycle_index
            }
            _ => subscription.cycle_index,
        };

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
        };

        patch.patch(conn).await?;

        if next_cycle.should_bill {
            self.bill_subscription_tx(
                conn,
                subscription.tenant_id,
                subscription.id,
                InvoiceBillingMode::AwaitGracePeriodIfApplicable,
            )
            .await?;
        }

        Ok(())
    }

    fn activate_subscription(&self, subscription: &SubscriptionRow) -> StoreResult<NextCycle> {
        if subscription.trial_duration.is_some() {
            let new_period_start = subscription
                .current_period_end
                .unwrap_or_else(|| Utc::now().naive_utc().date());
            let new_period_end = new_period_start
                .checked_add_days(Days::new(subscription.trial_duration.unwrap() as u64))
                .unwrap_or_else(|| new_period_start + Duration::days(7));
            Ok(NextCycle {
                status: SubscriptionStatusEnum::TrialActive,
                next_cycle_action: Some(CycleActionEnum::EndTrial),
                new_period_start,
                new_period_end: Some(new_period_end),
                should_bill: false, // Don't bill during trial activation - billing handled by trial_is_free logic
                pending_checkout: false,
            })
        } else {
            self.renew_subscription(subscription)
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
                .unwrap_or_else(|| Utc::now().naive_utc().date());

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
            let has_payment_method = subscription.payment_method.is_some();
            let can_auto_charge = subscription.charge_automatically && has_payment_method;

            // OnCheckout without auto-charge capability: require checkout before proceeding
            // Customer must complete checkout to activate the subscription
            if is_on_checkout && !can_auto_charge {
                let new_period_start = subscription
                    .current_period_end
                    .unwrap_or_else(|| Utc::now().naive_utc().date());

                return Ok(NextCycle {
                    status: SubscriptionStatusEnum::TrialExpired,
                    next_cycle_action: None,
                    new_period_start,
                    new_period_end: None,
                    should_bill: false,
                    pending_checkout: true,
                });
            }

            // For all other cases (OnStart, Manual, or OnCheckout with auto-charge):
            // - OnStart/Manual: subscription was already activated, just transition to billing
            // - OnCheckout with auto-charge: proceed to Active + charge
            // In all cases, the subscription becomes Active and an invoice is created.
            // For OnStart/Manual without auto-charge, the invoice is sent but payment is not
            // required for the subscription to continue (trust-based billing).
            self.renew_subscription(subscription)
        }
    }

    fn renew_subscription(&self, subscription: &SubscriptionRow) -> StoreResult<NextCycle> {
        // Calculate new period
        let new_period_start = subscription
            .current_period_end // cannot be null in this context
            .unwrap_or_else(|| Utc::now().naive_utc().date());

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
            .unwrap_or_else(|| Utc::now().naive_utc().date());

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

struct NextCycle {
    status: SubscriptionStatusEnum,
    next_cycle_action: Option<CycleActionEnum>,
    new_period_start: NaiveDate,
    new_period_end: Option<NaiveDate>,
    should_bill: bool,
    pending_checkout: bool,
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
    Utc::now().naive_utc() + Duration::minutes(delay_minutes) + Duration::seconds(jitter as i64)
}
