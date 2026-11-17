use crate::StoreResult;
use crate::domain::Subscription;
use crate::domain::enums::{BillingPeriodEnum, SubscriptionActivationCondition};
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::services::subscriptions::start::{StartTransition, mark_subscription_started_tx};
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Days, Duration, NaiveDate};
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::query::credit_inputs;
use diesel_models::subscriptions::SubscriptionRow;
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
    /// Starts a manually activated subscription as the lifecycle starts one on its start date: it
    /// opens cycle 0 and bills its first period (one-time fees included), unless a free trial
    /// defers billing to the trial's end; a paid trial bills now and ends on schedule.
    pub async fn activate_subscription_manual(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription> {
        let mut conn = self.store.get_conn().await?;
        let db_subscription = self
            .transaction_reading_usage_first(&mut conn, |services, conn| {
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    credit_inputs::bump_customer(conn, &lock).await?;

                    let subscription = services
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

                    let trial_days = subscription
                        .subscription
                        .trial_duration
                        .filter(|days| *days > 0);
                    // As creation and the lifecycle: the plan version says whether a trial is free.
                    let trial_is_free = match trial_days {
                        Some(_) => diesel_models::plans::PlanRow::get_with_version(
                            conn,
                            subscription.subscription.plan_version_id,
                            tenant_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .version
                        .is_none_or(|version| version.trial_is_free),
                        None => false,
                    };
                    let free_trial_days = trial_days.filter(|_| trial_is_free);
                    let paid_trial_days = trial_days.filter(|_| free_trial_days.is_none());

                    let (status, current_period_start, current_period_end, should_bill) =
                        if let Some(trial_duration) = free_trial_days {
                            let new_period_start = subscription
                                .subscription
                                .current_period_end
                                .unwrap_or_else(|| common_clock::now().naive_utc().date());
                            let new_period_end = new_period_start
                                .checked_add_days(Days::new(u64::from(trial_duration)))
                                .unwrap_or_else(|| new_period_start + Duration::days(7));

                            (
                                SubscriptionStatusEnum::TrialActive,
                                new_period_start,
                                Some(new_period_end),
                                false,
                            )
                        } else {
                            let new_period_start = subscription
                                .subscription
                                .billing_start_date
                                .unwrap_or(subscription.subscription.start_date);

                            let period = calculate_advance_period_range(
                                new_period_start,
                                subscription.subscription.billing_day_anchor as u32,
                                true,
                                &subscription.subscription.period,
                            );

                            (
                                if paid_trial_days.is_some() {
                                    SubscriptionStatusEnum::TrialActive
                                } else {
                                    SubscriptionStatusEnum::Active
                                },
                                period.start,
                                Some(period.end),
                                true,
                            )
                        };

                    mark_subscription_started_tx(
                        conn,
                        &*services.store.hooks.credit,
                        tenant_id,
                        subscription_id,
                        StartTransition::Activate {
                            current_period_start,
                            current_period_end,
                            status,
                            by: crate::services::subscriptions::start::StartSettledBy::Manual,
                        },
                    )
                    .await?;

                    if should_bill {
                        services
                            .bill_subscription_tx(
                                conn,
                                &crate::domain::entity_activity::Actor::System,
                                tenant_id,
                                subscription_id,
                                crate::services::InvoiceBillingMode::Immediate,
                            )
                            .await?;
                        services
                            .book_created_mrr_tx(conn, tenant_id, subscription_id)
                            .await?;
                    }

                    let updated =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    services
                        .emit_subscription_updated_tx(
                            conn,
                            tenant_id,
                            subscription_id,
                            SubscriptionUpdateType::Activated,
                        )
                        .await?;

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
    /// - No trial: activated with status Active
    /// - Free trial: activated with status TrialActive, its period the trial
    /// - Paid trial: activated with status TrialActive, its period a normal cycle
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
        let row = SubscriptionRow::get_subscription_by_id(conn, tenant_id, *subscription_id)
            .await?
            .subscription;

        let (status, current_period_start, current_period_end) =
            if row.status == SubscriptionStatusEnum::TrialExpired {
                // Its trial is over: paid, it starts its first period at the trial's end.
                let range = calculate_advance_period_range(
                    row.current_period_start,
                    params.billing_day_anchor,
                    true,
                    &params.period,
                );
                (SubscriptionStatusEnum::Active, range.start, Some(range.end))
            } else if has_trial && params.is_paid_trial {
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
                )
            } else {
                // No trial: normal billing
                let range = calculate_advance_period_range(
                    params.billing_start_date,
                    params.billing_day_anchor,
                    true,
                    &params.period,
                );

                (SubscriptionStatusEnum::Active, range.start, Some(range.end))
            };

        let billed = status == SubscriptionStatusEnum::Active || params.is_paid_trial;

        // The customer's payment method is resolved dynamically at billing time via payment_methods_config

        mark_subscription_started_tx(
            conn,
            &*self.store.hooks.credit,
            *tenant_id,
            *subscription_id,
            StartTransition::Activate {
                current_period_start,
                current_period_end,
                status,
                by: crate::services::subscriptions::start::StartSettledBy::Checkout,
            },
        )
        .await?;

        // Billed before activation; a free trial is not billed yet.
        if billed {
            self.book_created_mrr_tx(conn, *tenant_id, *subscription_id)
                .await?;
        }

        self.emit_subscription_updated_tx(
            conn,
            *tenant_id,
            *subscription_id,
            SubscriptionUpdateType::Activated,
        )
        .await?;

        Ok(())
    }
}
