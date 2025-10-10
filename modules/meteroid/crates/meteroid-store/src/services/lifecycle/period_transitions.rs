use crate::StoreResult;
use crate::services::{InvoiceBillingMode, Services};
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Days, Duration, NaiveDate, NaiveDateTime, Utc};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{
    ActionAfterTrialEnum, CycleActionEnum, ScheduledEventTypeEnum, SubscriptionStatusEnum,
};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscriptions::{
    SubscriptionCycleErrorRowPatch, SubscriptionCycleRowPatch, SubscriptionRow,
};

const BATCH_SIZE: i64 = 10;

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
                            log::error!(
                                "Error processing cycle transition for subscription {}: {:?}",
                                subscription.id,
                                err
                            );
                            // should we have a failure / errored terminal state ?
                            SubscriptionCycleErrorRowPatch {
                                id: subscription.id,
                                tenant_id: subscription.tenant_id,
                                last_error: Some(Some(err.to_string())),
                                next_retry: Some(Some(calculate_retry_time(
                                    subscription.error_count,
                                ))),
                                error_count: Some(subscription.error_count + 1),
                            }
                            .patch(tx)
                            .await?;
                        } else {
                            // TODO mark as processed / detect unchanged (avoid the risk of loops)
                            SubscriptionCycleErrorRowPatch {
                                id: subscription.id,
                                tenant_id: subscription.tenant_id,
                                last_error: Some(None),
                                next_retry: Some(None),
                                error_count: Some(0),
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
                        CycleActionEnum::EndTrial => self.end_trial(conn, subscription).await?,
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
                should_bill: true,
            })
        } else {
            self.renew_subscription(subscription)
        }
    }

    async fn end_trial(
        &self,
        conn: &mut PgConn,
        subscription: &SubscriptionRow,
    ) -> StoreResult<NextCycle> {
        let plan_version = PlanVersionRow::find_by_id_and_tenant_id(
            conn,
            subscription.plan_version_id,
            subscription.tenant_id,
        )
        .await?;

        let new_period_start = subscription
            .current_period_end // cannot be null in this context
            .unwrap_or_else(|| Utc::now().naive_utc().date());

        let period = calculate_advance_period_range(
            new_period_start,
            subscription.billing_day_anchor as u32,
            true,
            &(subscription.period.clone().into()),
        );

        match plan_version.action_after_trial {
            Some(ActionAfterTrialEnum::Charge) => {
                // we need to bill then activate
                Ok(NextCycle {
                    status: SubscriptionStatusEnum::PendingCharge,
                    next_cycle_action: Some(CycleActionEnum::RenewSubscription),
                    new_period_start,
                    new_period_end: Some(period.end),
                    should_bill: true,
                })
            }
            // even downgrade as it is to free plan (and it should be resolved via the plan_version.downgrade_plan_id)
            // TODO check & validate that downgrade is always to free plan
            None | Some(ActionAfterTrialEnum::Block | ActionAfterTrialEnum::Downgrade) => {
                Ok(NextCycle {
                    status: SubscriptionStatusEnum::TrialExpired,
                    next_cycle_action: None,
                    new_period_start,
                    new_period_end: Some(period.end),
                    should_bill: false,
                })
            }
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
        })
    }
}

struct NextCycle {
    status: SubscriptionStatusEnum,
    next_cycle_action: Option<CycleActionEnum>,
    new_period_start: NaiveDate,
    new_period_end: Option<NaiveDate>,
    should_bill: bool,
}

fn calculate_retry_time(retries: i32) -> NaiveDateTime {
    let delay_minutes = match retries {
        1 => 1,
        2 => 10,
        _ => 180,
    };

    let jitter = rand::random::<u64>() % 60; // up to 1 min
    Utc::now().naive_utc() + Duration::minutes(delay_minutes) + Duration::seconds(jitter as i64)
}
