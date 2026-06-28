use crate::data::ids::TENANT_ID;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{BaseId, ConnectorId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel::prelude::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use diesel_models::connectors::ConnectorRowNew;
use diesel_models::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use diesel_models::webhooks::WebhookInEventRow;
use meteroid_store::domain::pgmq::{PgmqQueue, WebhookInProcessEvent};
use meteroid_store::repositories::pgmq::PgmqInterface;

const ALIAS: &str = "stripe-webhook-test";

async fn post_event(client: &reqwest::Client, url: &str, event_id: &str) -> reqwest::StatusCode {
    // ~10 KB of padding so the body is well over the old 4 KB route limit; a 200
    // here also guards against the webhook body-size cap regressing.
    let padding = "x".repeat(10 * 1024);
    let body = serde_json::json!({
        "id": event_id,
        "object": "event",
        "type": "payment_intent.succeeded",
        "data": { "object": { "id": "pi_test", "description": padding } }
    });
    assert!(
        serde_json::to_vec(&body).unwrap().len() > 4096,
        "payload must exceed the old 4 KB limit to be a meaningful regression guard"
    );

    client.post(url).json(&body).send().await.unwrap().status()
}

/// Exercises the inbound webhook HTTP handler end to end: an unsigned Stripe
/// webhook is accepted, archived, persisted and enqueued, and a duplicate
/// delivery of the same event id is acked but deduped.
#[tokio::test]
async fn test_webhook_in_http_ingest_and_dedup() {
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    // A Stripe connector with no sensitive data, so signature verification is
    // skipped and the test can post an unsigned body.
    let connector_id = ConnectorId::new();
    {
        let mut conn = setup.store.pool.get().await.unwrap();
        ConnectorRowNew {
            id: connector_id,
            tenant_id: TENANT_ID,
            alias: ALIAS.to_string(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stripe,
            data: None,
            sensitive: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }

    let client = reqwest::Client::new();
    let url = format!(
        "{}/webhooks/v1/{}/{}",
        setup.config.rest_api_external_url, TENANT_ID, ALIAS
    );

    // First delivery is accepted.
    assert_eq!(
        post_event(&client, &url, "evt_http_a").await,
        reqwest::StatusCode::OK
    );
    // Duplicate delivery of the same event is also acked, but deduped.
    assert_eq!(
        post_event(&client, &url, "evt_http_a").await,
        reqwest::StatusCode::OK
    );
    // A distinct event produces a second record.
    assert_eq!(
        post_event(&client, &url, "evt_http_b").await,
        reqwest::StatusCode::OK
    );

    // Exactly one row per distinct event id (the duplicate was deduped), none
    // processed yet, each pointing at a non-empty object-store key.
    let rows: Vec<WebhookInEventRow> = {
        use diesel_models::schema::webhook_in_event::dsl as wi;
        let mut conn = setup.store.pool.get().await.unwrap();
        wi::webhook_in_event
            .filter(wi::provider_config_id.eq(connector_id.as_uuid()))
            .select(WebhookInEventRow::as_select())
            .load(&mut conn)
            .await
            .unwrap()
    };

    assert_eq!(rows.len(), 2, "one row per distinct event id");
    assert_eq!(
        rows.iter()
            .filter(|r| r.event_id.as_deref() == Some("evt_http_a"))
            .count(),
        1,
        "duplicate delivery must be deduped"
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r.event_id.as_deref() == Some("evt_http_b"))
            .count(),
        1
    );
    assert!(rows.iter().all(|r| r.processed_at.is_none()));
    assert!(rows.iter().all(|r| !r.key.is_empty()));

    // Each distinct event was enqueued exactly once for the worker; the
    // duplicate did not enqueue a second message.
    let messages = setup
        .store
        .pgmq_read(
            PgmqQueue::WebhookIn,
            MessageReadQty(10),
            MessageReadVtSec(5),
        )
        .await
        .unwrap();
    assert_eq!(messages.len(), 2, "one queue message per distinct event");

    let mut enqueued_ids: Vec<_> = messages
        .iter()
        .map(|m| {
            let ev: WebhookInProcessEvent = m.try_into().unwrap();
            ev.webhook_in_event_id
        })
        .collect();
    enqueued_ids.sort();
    let mut row_ids: Vec<_> = rows.iter().map(|r| r.id).collect();
    row_ids.sort();
    assert_eq!(enqueued_ids, row_ids, "queued ids match the persisted rows");
}
