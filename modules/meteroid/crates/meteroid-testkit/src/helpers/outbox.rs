use common_domain::ids::{BaseId, CustomerId, EventId, FeatureId, SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::pgmq::PgmqMessageRowNew;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::usage_alert_events::{AlertReading, EntitlementAlertEvent};

/// Publishes a beta `entitlement.limit_reached` event, as the usage-monitor engine does.
pub async fn publish_entitlement_limit_reached(conn: &mut PgConn, tenant_id: TenantId) {
    let event = OutboxEvent::EntitlementLimitReached(Box::new(EntitlementAlertEvent::new(
        EventId::new(),
        tenant_id,
        CustomerId::new(),
        FeatureId::new(),
        Some(SubscriptionId::new()),
        AlertReading {
            customer_alias: None,
            metric_code: Some("api_calls".to_owned()),
            group: None,
            threshold_pct: 100,
            used: rust_decimal::Decimal::from(1_000),
            limit: rust_decimal::Decimal::from(1_000),
            window_start: chrono::Utc::now().naive_utc(),
            window_end: None,
            as_of: chrono::Utc::now(),
        },
    )));
    let routing_key = event.routing_key();
    let row: PgmqMessageRowNew = event.try_into().expect("outbox row");
    diesel_models::query::pgmq::send_batch_topic(conn, routing_key, &[row], 0)
        .await
        .expect("publish outbox event");
}
