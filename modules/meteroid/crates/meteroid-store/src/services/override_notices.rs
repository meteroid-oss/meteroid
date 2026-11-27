//! `entitlement.override_ending_soon` and `entitlement.override_ended`: once per end of a customer
//! or resource override's lineage (R-148). An edit moving the end notices the new one; a revoked
//! override is never noticed.
use chrono::{DateTime, Utc};
use diesel_models::query::lifecycle_notices::{EndingOverrideRow, claim_ending_overrides};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::domain::outbox_event::OutboxEvent;
use crate::domain::usage_alert_events::OverrideEndingEvent;
use crate::errors::StoreError;
use crate::{Store, StoreResult};

/// How long before its end an override is noticed as ending soon.
pub const LEAD: chrono::Duration = chrono::Duration::days(7);

fn effect_name(effect: &str) -> String {
    match effect {
        "SET" => "set_to",
        "ADD" => "add",
        "RAISE_TO" => "raise_to",
        "CAP_AT" => "cap_at",
        other => other,
    }
    .to_string()
}

fn event(row: EndingOverrideRow, ended: bool) -> OutboxEvent {
    let event = Box::new(OverrideEndingEvent {
        id: OverrideEndingEvent::event_id(row.lineage_id, row.valid_until, ended),
        tenant_id: row.tenant_id,
        customer_id: row.customer_id,
        feature_id: row.feature_id,
        feature_code: row.feature_code,
        override_id: row.lineage_id,
        resource_id: row.resource_id,
        value: row.value,
        effect: effect_name(&row.effect),
        valid_until: row.valid_until,
        reason: row.reason,
    });
    if ended {
        OutboxEvent::EntitlementOverrideEnded(event)
    } else {
        OutboxEvent::EntitlementOverrideEndingSoon(event)
    }
}

/// Publishes the notices due at `now` and returns how many, ending soon then ended.
pub async fn notify_override_endings(
    store: &Store,
    now: DateTime<Utc>,
) -> StoreResult<(usize, usize)> {
    store
        .transaction(|conn| {
            async move {
                let ending = claim_ending_overrides(conn, now, LEAD, false)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let ended = claim_ending_overrides(conn, now, LEAD, true)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let counts = (ending.len(), ended.len());
                let events = ending
                    .into_iter()
                    .map(|row| event(row, false))
                    .chain(ended.into_iter().map(|row| event(row, true)))
                    .collect::<Vec<_>>();
                crate::repositories::outbox::publish_outbox_events_tx(conn, events).await?;
                Ok(counts)
            }
            .scope_boxed()
        })
        .await
}
