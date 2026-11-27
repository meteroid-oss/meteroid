//! A live subscription's end date moved, set or cleared (R-140): the day it stops renewing and
//! completes, recorded as its fixed term's fact. Only what is not billed yet can move: a date
//! cutting a period already billed is a cancellation on that date, which gives that period back.
use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::ScheduledEventTypeEnum;
use crate::domain::outbox_event::SubscriptionUpdateType;
use crate::domain::{Subscription, SubscriptionDetails, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::entity_activity::EntityActivityInterface;
use crate::services::Services;
use crate::services::evaluate::load::load_facts;
use crate::services::evaluate::pricer::StorePricer;
use crate::services::invoice_lines::fees::pricing_error;
use crate::store::PgConn;
use billing_core::charge::ChargeKey;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{BaseId, SubscriptionId, TenantId};
use diesel_models::query::credit_inputs;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

fn refused(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

fn conflict(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::Conflict(message.into()))
}

/// What the subscription has billed by `now` with its fixed term ending on `end_date`: each charge
/// due by then, by its key and the days it bills. Usage, arrears and the periods ahead are not due.
async fn due_charges(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    now: DateTime<Utc>,
    end_date: Option<NaiveDate>,
) -> StoreResult<Vec<(ChargeKey, (NaiveDate, NaiveDate))>> {
    let sub = &details.subscription;
    let mut loaded = load_facts(conn, sub.tenant_id, sub.id, now.date_naive(), None).await?;
    loaded.facts.end_date = end_date;
    let evaluation = billing_core::evaluate::evaluate(&loaded.facts, now, None, &StorePricer)
        .map_err(pricing_error)?;
    let mut due: Vec<_> = evaluation
        .charges
        .into_iter()
        .map(|charge| (charge.key, charge.window))
        .collect();
    due.sort();
    Ok(due)
}

/// Refuses an end date the rule refuses (R-140), with the subscription as it stands.
pub(in crate::services) async fn check_end_date(
    conn: &mut PgConn,
    details: &SubscriptionDetails,
    asked: Option<NaiveDate>,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    let sub = &details.subscription;
    let today = now.date_naive();
    if matches!(
        sub.status,
        SubscriptionStatusEnum::Cancelled
            | SubscriptionStatusEnum::Aborted
            | SubscriptionStatusEnum::Completed
    ) {
        return Err(conflict("The subscription has ended: its end date stays"));
    }
    if let Some(current) = sub.end_date
        && current <= today
    {
        return Err(conflict(format!(
            "The subscription completes on {current}: its end date is reached"
        )));
    }
    if let Some(asked) = asked {
        if asked <= today {
            return Err(refused(
                "An end date is after today: to end the subscription now, cancel it",
            ));
        }
        if asked < sub.start_date {
            return Err(refused("An end date is after the subscription's start"));
        }
    }
    // A cancellation waiting for its date ends the subscription first: revoked, the end date moves.
    if details
        .pending_events
        .iter()
        .any(|event| event.event_type == ScheduledEventTypeEnum::CancelSubscription)
    {
        return Err(conflict(
            "A cancellation is scheduled: revoke it before moving the end date",
        ));
    }
    // A change dated on or after the end would apply to no day (R-092).
    if let Some(asked) = asked
        && let Some(change) = details
            .pending_events
            .iter()
            .filter(|event| event.scheduled_date >= asked)
            .min_by_key(|event| event.scheduled_date)
    {
        return Err(conflict(format!(
            "A change is scheduled on {}, on or after the end date asked: withdraw it first",
            change.scheduled_date
        )));
    }
    // Only what is not billed yet moves: every charge due by now bills the same days either way.
    let before = due_charges(conn, details, now, sub.end_date).await?;
    let after = due_charges(conn, details, now, asked).await?;
    if before != after {
        let earlier = match (sub.end_date, asked) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(current), Some(asked)) => asked < current,
        };
        return Err(refused(match (sub.end_date, asked) {
            (_, Some(asked)) if earlier => format!(
                "An end date on {asked} cuts a period already billed: cancel the subscription on \
                 that date instead, which gives the time after it back"
            ),
            (Some(current), _) => format!(
                "The current end date, {current}, cuts a period already billed up to it: moving it \
                 would change what that period billed"
            ),
            (None, _) => "The end date would change what is already billed".to_owned(),
        }));
    }
    Ok(())
}

impl Services {
    /// Moves the end date of a live subscription to `end_date`, or clears it (`None`): it stops
    /// renewing and completes on that day, or renews until cancelled (R-140). Changing it to what
    /// it is changes nothing.
    pub(in crate::services) async fn change_subscription_end_date(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        end_date: Option<NaiveDate>,
    ) -> StoreResult<Subscription> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            self.store
                .transaction(|conn| {
                    let actor = &actor;
                    async move {
                        self.change_subscription_end_date_tx(
                            conn,
                            actor,
                            tenant_id,
                            subscription_id,
                            end_date,
                        )
                        .await
                    }
                    .scope_boxed()
                })
                .await
        })
        .await
    }

    /// [`Self::change_subscription_end_date`] in the caller's transaction, as a patch writing it
    /// beside other fields takes it.
    pub(in crate::services) async fn change_subscription_end_date_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        end_date: Option<NaiveDate>,
    ) -> StoreResult<Subscription> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            // Customer first, as every transaction writing the subscription or its facts.
            let lock = SubscriptionRow::lock_customer_of_tenant(conn, tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let details: SubscriptionDetails = self
                .store
                .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
                .await?;
            if details.subscription.end_date == end_date {
                return Ok(details.subscription);
            }
            let now = common_clock::now();
            check_end_date(conn, &details, end_date, now).await?;
            crate::services::subscriptions::phases::refuse_end_date_over_timeline_tx(
                conn,
                tenant_id,
                subscription_id,
                end_date,
            )
            .await?;
            // The subscription's life bounds the credit scoped to it.
            credit_inputs::bump_customer(conn, &lock).await?;

            crate::services::facts::record_end_date_tx(
                conn,
                tenant_id,
                subscription_id,
                end_date,
                actor,
            )
            .await?;

            let activity = Activity::new(
                ActivityType::EntityUpdated,
                EntityType::Subscription,
                subscription_id.as_uuid(),
            )
            .agg_customer(details.subscription.customer_id)
            .with_metadata(serde_json::json!({
                "changes": [{
                    "field": "end_date",
                    "from": details.subscription.end_date,
                    "to": end_date,
                }],
            }));
            self.store
                .record_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                .await?;
            self.emit_subscription_updated_tx(
                conn,
                tenant_id,
                subscription_id,
                SubscriptionUpdateType::BillingConfigurationUpdated,
            )
            .await?;

            // The controller wakes at the new end, or renews past the old one.
            self.derive_state_tx(conn, actor, tenant_id, subscription_id, now)
                .await?;

            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()
        })
        .await
    }
}
