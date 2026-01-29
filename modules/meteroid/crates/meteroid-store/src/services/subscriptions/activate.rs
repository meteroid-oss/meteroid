use crate::StoreResult;
use crate::domain::Subscription;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionActivationCondition};
use crate::domain::scheduled_events::ScheduledEventNew;
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Days, Duration, NaiveDate, Utc};
use common_domain::ids::{CustomerPaymentMethodId, SubscriptionId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{CycleActionEnum, PaymentMethodTypeEnum, SubscriptionStatusEnum};
use diesel_models::scheduled_events::ScheduledEventRowNew;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;

/// Parameters for activating a subscription after payment confirmation.
pub struct PaymentActivationParams {
    pub billing_start_date: NaiveDate,
    pub trial_duration: Option<i32>,
    pub is_paid_trial: bool,
    pub billing_day_anchor: u32,
    pub period: BillingPeriodEnum,
    /// Optional payment method to set during activation.
    pub payment_method: Option<PaymentMethodInfo>,
}

/// Payment method information to attach during activation.
pub struct PaymentMethodInfo {
    pub id: CustomerPaymentMethodId,
    pub method_type: PaymentMethodTypeEnum,
}

impl Services {
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

                    let subscription = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;

                    if subscription.subscription.activation_condition
                        != SubscriptionActivationCondition::Manual
                    {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Subscription activation condition must be Manual".to_string(),
                        )));
                    }

                    if subscription.subscription.activated_at.is_some() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Subscription is already activated".to_string(),
                        )));
                    }

                    // TODO check
                    // Calculate activation parameters based on trial
                    let (
                        status,
                        current_period_start,
                        current_period_end,
                        next_cycle_action,
                        cycle_index,
                    ) = if let Some(trial_duration) = subscription.subscription.trial_duration {
                        // Has trial: activate with trial status
                        let new_period_start = subscription
                            .subscription
                            .current_period_end
                            .unwrap_or_else(|| Utc::now().naive_utc().date());
                        let new_period_end = new_period_start
                            .checked_add_days(Days::new(trial_duration as u64))
                            .unwrap_or_else(|| new_period_start + Duration::days(7));

                        (
                            SubscriptionStatusEnum::TrialActive,
                            new_period_start,
                            Some(new_period_end),
                            Some(CycleActionEnum::EndTrial),
                            Some(0),
                        )
                    } else {
                        // No trial: activate directly to active state
                        let new_period_start = subscription
                            .subscription
                            .billing_start_date
                            .or(Some(subscription.subscription.start_date))
                            .unwrap_or_else(|| Utc::now().naive_utc().date());

                        let period = calculate_advance_period_range(
                            new_period_start,
                            subscription.subscription.billing_day_anchor as u32,
                            true,
                            &subscription.subscription.period,
                        );

                        (
                            SubscriptionStatusEnum::Active,
                            new_period_start,
                            Some(period.end),
                            Some(CycleActionEnum::RenewSubscription),
                            Some(1),
                        )
                    };

                    // Activate the subscription
                    SubscriptionRow::activate_subscription(
                        conn,
                        &subscription_id,
                        &tenant_id,
                        current_period_start,
                        current_period_end,
                        next_cycle_action,
                        cycle_index,
                        status,
                    )
                    .await?;

                    // Fetch and return updated subscription
                    let updated =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(updated)
                }
                .scope_boxed()
            })
            .await?;

        let subscription: Subscription = db_subscription.try_into()?;

        Ok(subscription)
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

        if let Some(pm) = params.payment_method {
            SubscriptionRow::activate_subscription_with_payment_method(
                conn,
                subscription_id,
                tenant_id,
                current_period_start,
                current_period_end,
                next_cycle_action,
                Some(0),
                status,
                Some(pm.id),
                Some(pm.method_type),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        } else {
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
        }

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
