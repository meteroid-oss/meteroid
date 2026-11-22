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
use crate::services::Services;
use crate::services::credit::scope::credit_ended_at;
use crate::services::credit::subscription_end::end_subscription_credit_tx;
use common_domain::ids::{BaseId, SubscriptionId, TenantId};

impl Services {
    pub(in crate::services) async fn cancel_subscription(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
    ) -> StoreResult<Subscription> {
        self.cancel_subscription_at(
            actor,
            subscription_id,
            tenant_id,
            reason,
            effective_at,
            common_clock::now(),
        )
        .await
    }

    /// The cancellation requested at `requested_at`, the clock or a replay's earlier moment: it
    /// applies in the request when its ending is reached by then.
    pub(in crate::services) async fn cancel_subscription_at(
        &self,
        actor: Actor,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        requested_at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<Subscription> {
        if requested_at > common_clock::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a cancellation cannot be requested at a future moment".to_string(),
            )));
        }
        let (db_subscription, ends_at) = self
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

                    // A cancellation replaces the changes still to come.
                    crate::services::facts::pending::withdraw_pending_of_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        crate::services::facts::pending::CHANGES,
                        "Replaced by subscription cancellation",
                    )
                    .await?;

                    let now = requested_at.naive_utc();

                    // A never-activated subscription (awaiting checkout / pending activation) was
                    // never counted in the active population: abort it immediately with no CANCELLED
                    // lifecycle event (which would wrongly decrement the active-subscriptions total).
                    if subscription.subscription.activated_at.is_none() {
                        SubscriptionCycleRowPatch {
                            id: subscription_id,
                            tenant_id,
                            status: Some(SubscriptionStatusEnum::Aborted),
                            current_period_start: None,
                            current_period_end: None,
                            pending_checkout: Some(false),
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
                                common_clock::now(),
                            )
                            .await?;
                        }

                        return Ok((res, None));
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
                                period_end_in_force(conn, &subscription, requested_at)
                                    .await?
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
                    let ends_at =
                        usage_cutoff.unwrap_or_else(|| billing_end_date.and_time(NaiveTime::MIN));

                    crate::services::facts::record_cancellation_tx(
                        conn,
                        tenant_id,
                        subscription_id,
                        ends_at,
                        now,
                        reason,
                        actor,
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
                        created_at: common_clock::now().naive_utc(),
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

                    // The controller wakes at the ending, before its period's end.
                    self.derive_state_tx(conn, actor, tenant_id, subscription_id, requested_at)
                        .await?;

                    Ok((res, Some(ends_at.and_utc())))
                }
                .scope_boxed()
            })
            .await?;

        // Reached already, the ending applies now, as the controller would: an ending past the
        // stored period's end first renews each period before it, one boundary a step.
        if ends_at.is_some_and(|at| at <= requested_at) {
            self.step_until_idle(
                tenant_id,
                subscription_id,
                crate::services::lifecycle::controller::CATCH_UP,
                crate::services::lifecycle::controller::StepOf::Request,
            )
            .await?;
            let mut conn = self.store.get_conn().await?;
            return SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into();
        }
        db_subscription.try_into()
    }
}

/// The end of the period the subscription is in at `at`, on its grid (R-084): the stored one,
/// unless the controller has not renewed past it yet (it is behind, or retrying), then the one its
/// facts hold at `at`. None without a period.
async fn period_end_in_force(
    conn: &mut crate::store::PgConn,
    details: &SubscriptionDetails,
    at: chrono::DateTime<chrono::Utc>,
) -> StoreResult<Option<NaiveDate>> {
    let stored = details.subscription.current_period_end;
    if stored.is_none_or(|end| end > at.date_naive()) {
        return Ok(stored);
    }
    let loaded = crate::services::evaluate::load::load_facts(
        conn,
        details.subscription.tenant_id,
        details.subscription.id,
        at.date_naive(),
        None,
    )
    .await?;
    let evaluation = billing_core::evaluate::evaluate(
        &loaded.facts,
        at,
        None,
        &crate::services::evaluate::pricer::StorePricer,
    )
    .map_err(crate::services::invoice_lines::fees::pricing_error)?;
    Ok(evaluation
        .state
        .current_period
        .and_then(|(_, end)| end)
        .filter(|end| *end > at.date_naive())
        .or(stored))
}
