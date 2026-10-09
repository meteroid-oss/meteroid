use crate::StoreResult;
use crate::domain::Subscription;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionActivationCondition};
use crate::domain::scheduled_events::ScheduledEventNew;
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::{InvoiceBillingMode, Services};
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use diesel_models::plans::PlanRow;
use diesel_models::scheduled_events::ScheduledEventRowNew;
use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Parameters for activating a subscription after payment confirmation.
pub struct PaymentActivationParams {
    pub billing_start_date: NaiveDate,
    pub trial_duration: Option<i32>,
    pub is_paid_trial: bool,
    pub billing_day_anchor: u32,
    pub period: BillingPeriodEnum,
}

impl Services {
    /// Billing starts on the activation day, or on the planned billing start if that is
    /// still ahead: time spent pending is never billed.
    pub async fn activate_subscription_manual(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription> {
        let db_subscription = self
            .store
            .transaction(|conn| {
                async move {
                    SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;

                    let sub = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?
                        .subscription;

                    if sub.activation_condition != SubscriptionActivationCondition::Manual {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Subscription activation condition must be Manual".to_string(),
                        )));
                    }

                    if sub.activated_at.is_some() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Subscription is already activated".to_string(),
                        )));
                    }

                    let today = Utc::now().naive_utc().date();
                    let planned_start = sub.billing_start_date.unwrap_or(sub.start_date);

                    if planned_start > today {
                        // Same state as an OnStart subscription with a future start date.
                        SubscriptionRow::activate_subscription(
                            conn,
                            &subscription_id,
                            &tenant_id,
                            planned_start,
                            Some(planned_start),
                            Some(CycleActionEnum::ActivateSubscription),
                            None,
                            SubscriptionStatusEnum::PendingActivation,
                        )
                        .await?;
                    } else {
                        let trial_is_free =
                            PlanRow::get_with_version(conn, sub.plan_version_id, tenant_id)
                                .await?
                                .version
                                .is_some_and(|v| v.trial_is_free);
                        let trial_duration = sub.trial_duration.filter(|&d| d > 0);
                        let has_free_trial = trial_duration.is_some() && trial_is_free;

                        let anchor_for = |start: NaiveDate| match trial_duration {
                            Some(days) if trial_is_free => {
                                (start + Duration::days(i64::from(days))).day()
                            }
                            _ => start.day(),
                        };
                        // Re-anchor a derived anniversary anchor; keep an explicit fixed day.
                        let billing_day_anchor =
                            if u32::from(sub.billing_day_anchor) == anchor_for(planned_start) {
                                anchor_for(today)
                            } else {
                                u32::from(sub.billing_day_anchor)
                            };

                        SubscriptionCycleRowPatch {
                            id: subscription_id,
                            tenant_id,
                            cycle_index: None,
                            status: None,
                            next_cycle_action: None,
                            current_period_start: None,
                            current_period_end: None,
                            pending_checkout: None,
                            processing_started_at: None,
                            billing_start_date: Some(today),
                            billing_day_anchor: Some(billing_day_anchor as i16),
                        }
                        .patch(conn)
                        .await?;

                        self.activate_subscription_after_payment(
                            conn,
                            &subscription_id,
                            &tenant_id,
                            PaymentActivationParams {
                                billing_start_date: today,
                                trial_duration: trial_duration.map(|d| d as i32),
                                is_paid_trial: !trial_is_free,
                                billing_day_anchor,
                                period: sub.period,
                            },
                        )
                        .await?;

                        if !has_free_trial {
                            self.bill_subscription_tx(
                                conn,
                                tenant_id,
                                subscription_id,
                                InvoiceBillingMode::Immediate,
                            )
                            .await?;
                        }
                    }

                    SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                }
                .scope_boxed()
            })
            .await?;

        db_subscription.try_into()
    }

    /// Activates a subscription after payment has been confirmed.
    ///
    /// Handles three subscription types:
    /// - No trial: activated with status Active and RenewSubscription cycle action
    /// - Free trial: activated with status TrialActive and EndTrial cycle action
    ///   (billing period = trial duration)
    /// - Paid trial: activated with status TrialActive and RenewSubscription cycle action
    ///   (billing period = normal cycle, trial end handled via scheduled event)
    ///
    /// If `payment_method` is provided, it will be set on the subscription during activation.
    pub async fn activate_subscription_after_payment(
        &self,
        conn: &mut PgConn,
        subscription_id: &SubscriptionId,
        tenant_id: &TenantId,
        params: PaymentActivationParams,
    ) -> Result<(), Report<StoreError>> {
        let has_trial = params.trial_duration.is_some_and(|d| d > 0);

        let (status, current_period_start, current_period_end, next_cycle_action) =
            if has_trial && params.is_paid_trial {
                // Paid trial: use normal billing cycle, trial end handled via scheduled event
                let range = calculate_advance_period_range(
                    params.billing_start_date,
                    params.billing_day_anchor,
                    true,
                    &params.period,
                );

                (
                    SubscriptionStatusEnum::TrialActive,
                    range.start,
                    Some(range.end),
                    Some(CycleActionEnum::RenewSubscription),
                )
            } else if has_trial {
                // Free trial: billing period = trial duration
                let trial_duration = params.trial_duration.unwrap_or(0);
                let period_end =
                    params.billing_start_date + chrono::Duration::days(i64::from(trial_duration));

                (
                    SubscriptionStatusEnum::TrialActive,
                    params.billing_start_date,
                    Some(period_end),
                    Some(CycleActionEnum::EndTrial),
                )
            } else {
                // No trial: normal billing
                let range = calculate_advance_period_range(
                    params.billing_start_date,
                    params.billing_day_anchor,
                    true,
                    &params.period,
                );

                (
                    SubscriptionStatusEnum::Active,
                    range.start,
                    Some(range.end),
                    Some(CycleActionEnum::RenewSubscription),
                )
            };

        // The customer's payment method is resolved dynamically at billing time via payment_methods_config

        SubscriptionRow::activate_subscription(
            conn,
            subscription_id,
            tenant_id,
            current_period_start,
            current_period_end,
            next_cycle_action,
            Some(0),
            status,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        // For paid trials, schedule the EndTrial event to transition status when trial ends
        if has_trial
            && params.is_paid_trial
            && let Some(trial_days) = params.trial_duration
        {
            let scheduled_event = ScheduledEventNew::end_trial(
                *subscription_id,
                *tenant_id,
                params.billing_start_date,
                trial_days,
                "payment_activation",
            )
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "Failed to compute trial end date".to_string(),
                ))
            })?;
            let insertable: ScheduledEventRowNew = scheduled_event.try_into()?;
            ScheduledEventRowNew::insert_batch(conn, &[insertable])
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        }

        Ok(())
    }
}
