use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::SubscriptionEventType;
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::domain::{Subscription, SubscriptionDetails};
use crate::errors::StoreError;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use diesel_models::query::credit_inputs;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use uuid::Uuid;

use crate::repositories::SubscriptionInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};
// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant
use crate::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use crate::services::Services;
use crate::services::credit::scope::credit_ended_at;
use crate::services::credit::subscription_end::end_subscription_credit_tx;
use common_domain::ids::{BaseId, SubscriptionId, TenantId};
use diesel_models::scheduled_events::ScheduledEventRow;

impl Services {
    pub(in crate::services) async fn cancel_subscription(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<Subscription> {
        let db_subscription = self
            .store
            .transaction(|conn| {
                let actor = &actor;
                let reason = reason.clone();
                async move {
                    // Customer first, as every transaction writing the subscription or its events.
                    let lock =
                        SubscriptionRow::lock_customer_of_tenant(conn, tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    credit_inputs::bump_customer(conn, &lock).await?;
                    let subscription: SubscriptionDetails = self
                        .store
                        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                        .await?;
                    let customer_id = subscription.subscription.customer_id;
                    let reason_for_audit = reason.clone();

                    // Cancel all pending lifecycle events before scheduling the new cancellation
                    ScheduledEventRow::cancel_pending_lifecycle_events(
                        conn,
                        subscription_id,
                        &tenant_id,
                        "Replaced by subscription cancellation",
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let now = common_domain::clock::now().naive_utc();

                    // A never-activated subscription (awaiting checkout / pending activation) was
                    // never counted in the active population: abort it immediately with no CANCELLED
                    // lifecycle event (which would wrongly decrement the active-subscriptions total).
                    if subscription.subscription.activated_at.is_none() {
                        SubscriptionCycleRowPatch {
                            id: subscription_id,
                            tenant_id,
                            status: Some(SubscriptionStatusEnum::Aborted),
                            next_cycle_action: Some(None),
                            cycle_index: None,
                            current_period_start: None,
                            current_period_end: None,
                            pending_checkout: Some(false),
                            processing_started_at: Some(None),
                            billing_start_date: None,
                            billing_day_anchor: None,
                            terminated_at: None,
                        }
                        .patch(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        // Kill any still-open checkout session so a late payment can't resurrect
                        // (re-activate + bill) the aborted subscription.
                        CheckoutSessionRow::cancel_open_for_subscription(
                            conn,
                            tenant_id,
                            subscription_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        let activity = {
                            let mut a = Activity::new(
                                ActivityType::SubscriptionCancelled,
                                EntityType::Subscription,
                                subscription_id.as_uuid(),
                            )
                            .agg_customer(customer_id);
                            if let Some(reason) = reason_for_audit {
                                a = a.with_metadata(serde_json::json!({ "reason": reason }));
                            }
                            a
                        };
                        self.store
                            .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;

                        self.emit_subscription_cancelled_tx(
                            conn,
                            tenant_id,
                            subscription_id,
                            reason.clone(),
                        )
                        .await?;

                        let res = SubscriptionRow::get_subscription_by_id(
                            conn,
                            &tenant_id,
                            subscription_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        if let Some(ended_at) = credit_ended_at(&res.subscription) {
                            end_subscription_credit_tx(
                                conn,
                                &*self.store.hooks.credit,
                                tenant_id,
                                subscription_id,
                                ended_at,
                                common_domain::clock::now(),
                            )
                            .await?;
                        }

                        return Ok(res);
                    }

                    // `usage_cutoff` is set only for an immediate cancel: it pins the terminal
                    // period's usage window to the request instant, so same-day usage is billed
                    // rather than truncated at the effective date's midnight. A date-effective
                    // cancel keeps the midnight boundary (None).
                    let (mut billing_end_date, usage_cutoff): (NaiveDate, Option<NaiveDateTime>) =
                        match effective_at {
                            // The period the subscription is in ends it, as Stripe's
                            // cancel_at_period_end: counting a cadence from today overshoots it.
                            CancellationEffectiveAt::EndOfBillingPeriod => (
                                subscription
                                    .subscription
                                    .current_period_end
                                    .or_else(|| {
                                        subscription
                                            .calculate_cancellable_end_of_period_date(now.date())
                                    })
                                    .ok_or(Report::from(StoreError::CancellationError))?,
                                None,
                            ),
                            CancellationEffectiveAt::Date(date) => (date, None),
                            CancellationEffectiveAt::Immediate => (now.date(), Some(now)),
                        };

                    // A fixed-term subscription can't be cancelled effective after its natural end:
                    // clamp so the cancellation (-1) and the natural completion (Ended -1) can't both
                    // land, which would double-decrement the active-subscriptions count.
                    if let Some(end_date) = subscription.subscription.end_date {
                        billing_end_date = billing_end_date.min(end_date);
                    }

                    // Keep the instant cutoff only when it lands on the (possibly clamped) settled
                    // day. A fixed-term sub cancelled after its end_date clamps billing_end_date into
                    // the past, so `now` no longer belongs to the settled period; fall back to the
                    // date's midnight, keeping `terminated_at` consistent with the period boundary.
                    let usage_cutoff = usage_cutoff.filter(|c| c.date() == billing_end_date);

                    self.store
                        .schedule_events(
                            conn,
                            vec![ScheduledEventNew {
                                subscription_id: subscription.subscription.id,
                                tenant_id,
                                scheduled_time: billing_end_date.and_time(NaiveTime::MIN),
                                event_data: ScheduledEventData::CancelSubscription {
                                    reason,
                                    usage_cutoff,
                                },
                                source: "edge".to_string(),
                                created_by: actor.clone(),
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
                        details: None,
                        created_at: common_domain::clock::now().naive_utc(),
                        mrr_delta: Some(-(mrr as i64)),
                        bi_mrr_movement_log_id: None,
                        applies_to: billing_end_date,
                    };

                    event
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let activity = {
                        let mut a = Activity::new(
                            ActivityType::SubscriptionCancellationScheduled,
                            EntityType::Subscription,
                            subscription_id.as_uuid(),
                        )
                        .agg_customer(customer_id);
                        if let Some(reason) = reason_for_audit {
                            a = a.with_metadata(serde_json::json!({ "reason": reason }));
                        }
                        a
                    };
                    self.store
                        .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;

                    self.emit_subscription_updated_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        SubscriptionUpdateType::CancellationScheduled,
                    )
                    .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        db_subscription.try_into()
    }
}
