use crate::StoreResult;
use crate::domain::enums::SubscriptionEventType;
use crate::domain::{Subscription, SubscriptionDetails};
use crate::errors::StoreError;
use chrono::{NaiveDate, NaiveTime};
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::Report;
use uuid::Uuid;

use crate::repositories::SubscriptionInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;
// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant
use crate::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use crate::services::Services;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::scheduled_events::ScheduledEventRow;

impl Services {
    pub(in crate::services) async fn cancel_subscription(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        _actor: Uuid,
    ) -> StoreResult<Subscription> {
        let db_subscription = self
            .store
            .transaction(|conn| {
                async move {
                    let subscription: SubscriptionDetails = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;

                    // Cancel all pending lifecycle events before scheduling the new cancellation
                    ScheduledEventRow::cancel_pending_lifecycle_events(
                        conn,
                        subscription_id,
                        &tenant_id,
                        "Replaced by subscription cancellation",
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let now = chrono::Utc::now().naive_utc();

                    let billing_end_date: NaiveDate = match effective_at {
                        CancellationEffectiveAt::EndOfBillingPeriod => subscription
                            .calculate_cancellable_end_of_period_date(now.date())
                            .ok_or(Report::from(StoreError::CancellationError))?,
                        CancellationEffectiveAt::Date(date) => date,
                    };

                    self.store
                        .schedule_events(
                            conn,
                            vec![ScheduledEventNew {
                                subscription_id: subscription.subscription.id,
                                tenant_id,
                                scheduled_time: billing_end_date.and_time(NaiveTime::MIN),
                                event_data: ScheduledEventData::CancelSubscription { reason },
                                source: "edge".to_string(), // TODO drop
                            }],
                        )
                        .await?;

                    let res =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    let mrr = subscription.subscription.mrr_cents;

                    let event = SubscriptionEventRow {
                        id: Uuid::now_v7(),
                        subscription_id,
                        event_type: SubscriptionEventType::Cancelled.into(),
                        details: None, // TODO reason etc
                        created_at: chrono::Utc::now().naive_utc(),
                        mrr_delta: Some(-(mrr as i64)),
                        bi_mrr_movement_log_id: None,
                        applies_to: billing_end_date,
                    };

                    event
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        let subscription: Subscription = db_subscription.try_into()?;

        // let _ = self
        //     .store
        //     .eventbus
        //     .publish(Event::subscription_canceled(
        //         actor,
        //         subscription.id.as_uuid(),
        //         subscription.tenant_id.as_uuid(),
        //     ))
        //     .await;

        Ok(subscription)
    }
}
