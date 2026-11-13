use crate::StoreResult;
use crate::domain::outbox_event;
use crate::errors::StoreError;
use crate::store::{PgConn, StoreInternal};
use diesel_models::pgmq::PgmqMessageRowNew;
use diesel_models::query::pgmq;
use error_stack::Report;

impl StoreInternal {
    /// Publishes events to their topic routing keys in-tx, in order: one `send_batch_topic`
    /// call per run of consecutive events sharing a key.
    pub async fn publish_outbox_events_topic_tx(
        &self,
        conn: &mut PgConn,
        events: Vec<outbox_event::OutboxEvent>,
    ) -> StoreResult<()> {
        publish_outbox_events_tx(conn, events).await
    }
}

/// `publish_outbox_events_topic_tx` for writers that hold only a connection, such as the ledger.
pub async fn publish_outbox_events_tx(
    conn: &mut PgConn,
    events: Vec<outbox_event::OutboxEvent>,
) -> StoreResult<()> {
    for (routing_key, batch) in batches_in_order(events)? {
        let delivered = pgmq::send_batch_topic(conn, routing_key, &batch, 0)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if delivered == 0 {
            log::warn!(
                "outbox topic publish for `{routing_key}` ({} message(s)) matched no queue bindings",
                batch.len()
            );
        }
    }

    Ok(())
}

/// One batch per run of consecutive events sharing a routing key, so consumers receive the
/// events in the order they were given (a 50% alert before the 100% one of the same crossing).
fn batches_in_order(
    events: Vec<outbox_event::OutboxEvent>,
) -> StoreResult<Vec<(&'static str, Vec<PgmqMessageRowNew>)>> {
    let mut batches: Vec<(&'static str, Vec<PgmqMessageRowNew>)> = Vec::new();
    for event in events {
        let routing_key = event.routing_key();
        let row: PgmqMessageRowNew = event.try_into()?;
        match batches.last_mut() {
            Some((key, batch)) if *key == routing_key => batch.push(row),
            _ => batches.push((routing_key, vec![row])),
        }
    }
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use super::batches_in_order;
    use crate::domain::outbox_event::OutboxEvent;
    use crate::domain::usage_alert_events::{AlertReading, BudgetAlertEvent};
    use common_domain::ids::{BaseId, BudgetId, CustomerId, EventId, TenantId};
    use rust_decimal_macros::dec;

    fn budget_alert(threshold_pct: u32) -> OutboxEvent {
        OutboxEvent::budget_alert(BudgetAlertEvent::new(
            EventId::new(),
            TenantId::new(),
            CustomerId::new(),
            BudgetId::new(),
            "EUR".to_owned(),
            None,
            AlertReading {
                customer_alias: None,
                metric_code: None,
                group: None,
                threshold_pct,
                used: dec!(12000),
                limit: dec!(10000),
                window_start: chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                window_end: None,
                as_of: common_domain::clock::now(),
            },
        ))
    }

    #[test]
    fn events_are_batched_in_the_order_they_are_given() {
        let batches = batches_in_order(vec![
            budget_alert(50),
            budget_alert(80),
            budget_alert(100),
            budget_alert(50),
        ])
        .unwrap();
        let keys = batches
            .iter()
            .map(|(key, batch)| (*key, batch.len()))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                (BudgetAlertEvent::THRESHOLD_ROUTING_KEY, 2),
                (BudgetAlertEvent::EXCEEDED_ROUTING_KEY, 1),
                (BudgetAlertEvent::THRESHOLD_ROUTING_KEY, 1),
            ]
        );
    }
}
