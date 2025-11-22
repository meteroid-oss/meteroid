use crate::StoreResult;
use crate::domain::Subscription;
use crate::domain::enums::SubscriptionActivationCondition;
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Days, Duration, Utc};
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{CycleActionEnum, SubscriptionStatusEnum};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;

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
                    // Lock subscription for update
                    SubscriptionRow::lock_subscription_for_update(conn, subscription_id).await?;

                    // Fetch subscription details
                    let subscription = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;

                    // Validate activation condition
                    if subscription.subscription.activation_condition != SubscriptionActivationCondition::Manual {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Subscription activation condition must be Manual".to_string(),
                        )));
                    }

                    // Validate not already activated
                    if subscription.subscription.activated_at.is_some() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Subscription is already activated".to_string(),
                        )));
                    }


                    // TODO check
                    // Calculate activation parameters based on trial
                    let (status, current_period_start, current_period_end, next_cycle_action, cycle_index) =
                        if let Some(trial_duration) = subscription.subscription.trial_duration {
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
                    let updated = SubscriptionRow::get_subscription_by_id(
                        conn,
                        &tenant_id,
                        subscription_id,
                    )
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
}
