//! `subscription.trial_ending_soon`: once per trial end, `LEAD_DAYS` ahead of it (R-158). A trial
//! whose end moves is noticed again for its new end.
use chrono::{DateTime, Utc};
use diesel_models::query::lifecycle_notices::claim_ending_trials;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::domain::outbox_event::OutboxEvent;
use crate::domain::usage_alert_events::TrialEndingEvent;
use crate::errors::StoreError;
use crate::{Store, StoreResult};

/// Days before its end a trial is noticed, as Stripe's `trial_will_end`.
pub const LEAD_DAYS: i32 = 3;

/// Publishes the notices due at `now` and returns how many.
pub async fn notify_trial_endings(store: &Store, now: DateTime<Utc>) -> StoreResult<usize> {
    store
        .transaction(|conn| {
            async move {
                let due = claim_ending_trials(conn, now, now.date_naive(), LEAD_DAYS)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let count = due.len();
                let events = due
                    .into_iter()
                    .map(|row| {
                        OutboxEvent::SubscriptionTrialEndingSoon(Box::new(TrialEndingEvent {
                            id: TrialEndingEvent::event_id(row.subscription_id, row.until_date),
                            tenant_id: row.tenant_id,
                            customer_id: row.customer_id,
                            subscription_id: row.subscription_id,
                            trial_end: row.until_date,
                            free: row.free,
                            trial_offer_id: row.trial_offer_id,
                        }))
                    })
                    .collect::<Vec<_>>();
                crate::repositories::outbox::publish_outbox_events_tx(conn, events).await?;
                Ok(count)
            }
            .scope_boxed()
        })
        .await
}
