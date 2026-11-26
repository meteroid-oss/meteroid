use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::ScheduledEventTypeEnum;
use crate::domain::{AppliedCouponDetailed, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::services::Services;
use crate::services::subscriptions::cancel::period_end_in_force;
use crate::services::subscriptions::utils::apply_coupons_without_validation_at;
use common_domain::ids::{AppliedCouponId, BaseId, CouponId, SubscriptionId, TenantId};
use diesel_models::applied_coupons::{AppliedCouponDetailedRow, AppliedCouponRowNew};
use diesel_models::query::credit_inputs;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

impl Services {
    /// Adds a coupon to a running subscription. It discounts from the next billing period on: the
    /// current one is already invoiced or committed.
    pub(in crate::services) async fn apply_coupon_to_subscription(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        coupon_id: CouponId,
    ) -> StoreResult<AppliedCouponDetailed> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let lock =
                        SubscriptionRow::lock_customer_of_tenant(conn, tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    let details: SubscriptionDetails = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;
                    let subscription = &details.subscription;
                    if subscription.status.is_terminal() || subscription.activated_at.is_none() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "a coupon can only be added to an active subscription".to_string(),
                        )));
                    }
                    // It would start the day the subscription ends, or on a plan not checked yet.
                    if details.pending_events.iter().any(|e| {
                        matches!(
                            e.event_type,
                            ScheduledEventTypeEnum::CancelSubscription
                                | ScheduledEventTypeEnum::ApplyPlanChange
                        )
                    }) {
                        return Err(Report::new(StoreError::FailedPrecondition(
                            "the subscription has a pending cancellation or plan change"
                                .to_string(),
                        )));
                    }

                    self.lock_and_validate_coupons_for_checkout(
                        conn,
                        tenant_id,
                        subscription.customer_id,
                        &[coupon_id],
                        &subscription.currency,
                        Some(subscription.plan_version_id),
                        Some(subscription_id),
                        None,
                    )
                    .await?;

                    let now = common_clock::now();
                    let starts = coupon_start(conn, &details, now).await?;
                    if subscription.end_date.is_some_and(|end| end <= starts) {
                        return Err(Report::new(StoreError::FailedPrecondition(
                            "the subscription ends before the coupon would start".to_string(),
                        )));
                    }
                    let applied = AppliedCouponRowNew {
                        id: AppliedCouponId::new(),
                        subscription_id,
                        coupon_id,
                        customer_id: subscription.customer_id,
                        is_active: true,
                        applied_amount: None,
                        applied_count: None,
                        last_applied_at: None,
                        effective_from: starts,
                        discounts_from: Some(starts),
                    };
                    apply_coupons_without_validation_at(
                        conn,
                        &*self.store.hooks.credit,
                        &[&applied],
                        tenant_id,
                        now,
                    )
                    .await?;

                    self.store
                        .record_tx(
                            conn,
                            tenant_id,
                            actor,
                            AuditInput::Activity(
                                Activity::new(
                                    ActivityType::SubscriptionCouponApplied,
                                    EntityType::Subscription,
                                    subscription_id.as_uuid(),
                                )
                                .agg_customer(subscription.customer_id)
                                .with_metadata(serde_json::json!({
                                    "coupon_id": coupon_id.to_string(),
                                    "effective_from": applied.effective_from,
                                })),
                            ),
                        )
                        .await?;

                    AppliedCouponDetailedRow::list_by_subscription_id(conn, &subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into_iter()
                        .find(|row| row.applied_coupon.id == applied.id)
                        .ok_or_else(|| {
                            Report::new(StoreError::ValueNotFound(
                                "applied coupon not found".to_string(),
                            ))
                        })?
                        .try_into()
                }
                .scope_boxed()
            })
            .await
    }
}

/// The day a coupon added at `at` starts discounting: the next period's start.
pub(in crate::services) async fn coupon_start(
    conn: &mut crate::store::PgConn,
    details: &SubscriptionDetails,
    at: chrono::DateTime<chrono::Utc>,
) -> StoreResult<chrono::NaiveDate> {
    Ok(period_end_in_force(conn, details, at)
        .await?
        .unwrap_or_else(|| at.date_naive()))
}
