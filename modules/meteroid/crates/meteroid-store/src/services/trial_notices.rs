//! `subscription.trial_ending_soon`: once per trial end, `TRIAL_NOTICE_DAYS` ahead of it, at its
//! start when it is shorter (R-158). A trial whose end moves is noticed again for its new end.
use chrono::{DateTime, Utc};
use diesel_models::query::lifecycle_notices::claim_ending_trials;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::domain::outbox_event::OutboxEvent;
use crate::domain::trials::{TrialEnd, TrialWithoutCard};
use crate::domain::usage_alert_events::TrialEndingEvent;
use crate::errors::StoreError;
use crate::{Store, StoreResult};

pub use crate::domain::trials::TRIAL_NOTICE_DAYS as LEAD_DAYS;

/// Publishes the notices due at `now` and returns how many.
pub async fn notify_trial_endings(store: &Store, now: DateTime<Utc>) -> StoreResult<usize> {
    store
        .transaction(|conn| {
            async move {
                let due = claim_ending_trials(conn, now, now.date_naive(), LEAD_DAYS)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let count = due.len();
                let mut events = Vec::with_capacity(count);
                for row in due {
                    let lacks = store
                        .lacks_payment_method_tx(
                            conn,
                            row.tenant_id,
                            row.subscription_id,
                            row.charge_automatically,
                        )
                        .await?;
                    let end = TrialEnd::parse(&row.end_action);
                    let cancels_without_card = lacks
                        && row.free
                        && TrialWithoutCard::parse(&row.without_card, None)
                            == Some(TrialWithoutCard::Cancel);
                    let billing_starts_on = match end {
                        TrialEnd::Cancel | TrialEnd::Manual => None,
                        TrialEnd::Convert if cancels_without_card => None,
                        TrialEnd::Convert | TrialEnd::Fallback => Some(row.until_date),
                    };
                    crate::services::customer_emails::trials::enqueue_trial_ending_email_tx(
                        conn,
                        row.tenant_id,
                        row.customer_id,
                        row.subscription_id,
                        row.until_date,
                    )
                    .await?;
                    events.push(OutboxEvent::SubscriptionTrialEndingSoon(Box::new(
                        TrialEndingEvent {
                            id: TrialEndingEvent::event_id(row.subscription_id, row.until_date),
                            tenant_id: row.tenant_id,
                            customer_id: row.customer_id,
                            subscription_id: row.subscription_id,
                            trial_end: row.until_date,
                            free: row.free,
                            trial_offer_id: row.trial_offer_id,
                            billing_starts_on,
                            has_payment_method: Some(!lacks),
                            end_action: Some(row.end_action),
                            without_card: Some(row.without_card),
                        },
                    )));
                }
                crate::repositories::outbox::publish_outbox_events_tx(conn, events).await?;
                Ok(count)
            }
            .scope_boxed()
        })
        .await
}
