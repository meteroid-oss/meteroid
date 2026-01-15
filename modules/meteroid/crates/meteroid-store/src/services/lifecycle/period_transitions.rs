use crate::StoreResult;
use crate::services::{InvoiceBillingMode, Services};
use crate::store::PgConn;
use crate::utils::errors::format_error_chain;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Days, Duration, NaiveDate, NaiveDateTime, Utc};
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

const BATCH_SIZE: i64 = 10;
const MAX_CYCLE_RETRIES: i32 = 10;

impl Services {
    pub async fn get_and_process_cycle_transitions(&self) -> StoreResult<usize> {
        let len = self
            .store
            .transaction(|tx| {
                async move {
                    // Fetch events to process
                    let due_subscriptions =
                        SubscriptionRow::get_due_subscription_for_update(tx, BATCH_SIZE).await?;

                    for subscription in &due_subscriptions {
                        // Record lag TODO
                        // let delay = Utc::now().naive_utc().signed_duration_since(subscription.next_cycle_date);
                        // self.metrics.record_processing_delay(&subscription.next_cycle_action.to_string(), delay.num_seconds());

                        // Process the cycle transition
                        if let Err(err) = self.process_cycle_transition(tx, subscription).await {
                            let new_error_count = subscription.error_count + 1;
                            let error_message = format_error_chain(&err);

                            // Check if we've exceeded max retries
                            let (status, next_retry) = if new_error_count >= MAX_CYCLE_RETRIES {
                                log::error!(
                                    "Subscription {} exceeded max retries ({}), marking as Errored. Error: {}",
                                    subscription.id,
                                    MAX_CYCLE_RETRIES,
                                    error_message
                                );
                                (
                                    Some(SubscriptionStatusEnum::Errored),
                                    Some(None), // Clear next_retry since we're done retrying
                                )
                            } else {
                                log::warn!(
                                    "Error processing cycle transition for subscription {} (attempt {}/{}): {}",
                                    subscription.id,
                                    new_error_count,
                                    MAX_CYCLE_RETRIES,
                                    error_message
                                );
                                (
                                    None, // Don't change status yet
                                    Some(Some(calculate_retry_time(new_error_count))),
                                )
                            };

                            SubscriptionCycleErrorRowPatch {
                                id: subscription.id,
                                tenant_id: subscription.tenant_id,
                                last_error: Some(Some(error_message)),
                                next_retry,
                                error_count: Some(new_error_count),
                                status,
                            }
                            .patch(tx)
                            .await?;
                        } else {
                            SubscriptionCycleErrorRowPatch {
                                id: subscription.id,
                                tenant_id: subscription.tenant_id,
                                last_error: Some(None),
                                next_retry: Some(None),
                                error_count: Some(0),
                                status: None,
                            }
                            .patch(tx)
                            .await?;
                        }
                    }

                    Ok(due_subscriptions.len())
                }
                .scope_boxed()
            })
            .await?;

        Ok(len)
    }

    //  Executes the current scheduled action for a subscription (like generating an invoice, activating a subscription, ending a trial, etc.)
    async fn process_cycle_transition(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
    ) -> StoreResult<()> {
        self.store
            .transaction_with(conn, |conn| {
                async move {
                    // filter terminal states just in case ?
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
                        CycleActionEnum::ActivateSubscription => {
                            self.activate_subscription(subscription)?
                        }
                        CycleActionEnum::RenewSubscription => {
                            self.renew_subscription(subscription)?
                        }
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
                                // ScheduledEventTypeEnum::SuspendForNonPayment,
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

                    let patch = SubscriptionCycleRowPatch {
                        id: subscription.id,
                        tenant_id: subscription.tenant_id,
                        status: Some(next_cycle.status),
                        next_cycle_action: Some(next_cycle.next_cycle_action),
                        current_period_start: Some(next_cycle.new_period_start),
                        current_period_end: Some(next_cycle.new_period_end),
                        cycle_index: subscription.cycle_index.map(|i| i + 1),
                        pending_checkout: Some(next_cycle.pending_checkout),
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
                .scope_boxed()
            })
            .await?;

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

    /// Handles the end of a trial period.
    ///
    /// Business logic:
    /// - Free plan: → Active (no invoice ever)
    /// - Paid plan + has payment method: → Active + invoice
    /// - Paid plan + no payment method: → TrialExpired (awaiting checkout)
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
            // Paid plan: determine next action based on activation condition and payment setup
            let is_on_checkout = subscription.activation_condition
                == SubscriptionActivationConditionEnum::OnCheckout;
            let has_payment_method = subscription.payment_method.is_some();
            let can_auto_charge = subscription.charge_automatically && has_payment_method;

            // OnCheckout without auto-charge: always require checkout before invoice
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

            // Can auto-charge (has payment method + charge_automatically): proceed to Active
            if can_auto_charge {
                return self.renew_subscription(subscription);
            }

            // OnStart without payment method: create invoice for external payment
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
                status: SubscriptionStatusEnum::TrialExpired,
                next_cycle_action: None,
                new_period_start,
                new_period_end: Some(period.end),
                should_bill: true,
                pending_checkout: false,
            })
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
            true, // Align to billing_day_anchor (works for both first post-trial and renewals)
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
