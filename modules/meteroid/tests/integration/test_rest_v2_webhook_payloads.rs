//! Webhook payloads in API v2's model (ADR-0013 §15): an endpoint holding `2026-12-15` receives
//! envelopes `{id, type, created_at, api_version, data}` whose `data` is what the resource's v2 GET
//! answers; one holding no version, or `2026-12-01`, keeps v1's payloads byte for byte; a retry
//! and a resend send the same bytes; a patch of `api_version` changes the shape from the next
//! event on; an endpoint created from the dashboard holds the latest version. A notification
//! about credit carries its wallet, movement and grant (a checkout kept as credit its session)
//! as their v2 GETs read them, a retention notice its subscription; the dashboard's catalog and
//! examples follow the endpoint's payloads.
use crate::data::ids::TENANT_ID;
use crate::helpers::credit::expire_grant;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::test_rest_v2::{Fixture, fixture};
use crate::test_rest_v2_subscriptions::seed;
use chrono::Utc;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CreditGrantId, CustomerId, EventId, PaymentTransactionId,
    RetentionSessionId, SubscriptionId, WebhookEndpointId,
};
use diesel_async::RunQueryDsl;
use meteroid::workers::pgmq::processors;
use meteroid::workers::webhook_delivery::WebhookDeliveryWorker;
use meteroid_grpc::meteroid::api::webhooks::out::v1 as grpc;
use meteroid_store::domain::outbox_event::{
    CheckoutPaymentKeptAsCreditEvent, OutboxEvent, TopUpCapPeriod, WalletAutoTopUpCapReachedEvent,
};
use meteroid_store::domain::retention_events::{
    RenewalMode, RenewalUpcomingEvent, RetentionOfferKind, RetentionOutcome, RetentionSessionEvent,
};
use meteroid_store::domain::webhooks_out::{WebhookEndpoint, WebhookEndpointNew};
use meteroid_store::repositories::CustomerCreditWalletsInterface;
use meteroid_store::repositories::WebhookOutInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::DebitCustomerCreditGrant;
use meteroid_store::repositories::outbox::publish_outbox_events_tx;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

const V2: &str = "2026-12-15";

#[derive(Clone, Debug)]
struct Received {
    path: String,
    id: String,
    signature: String,
    body: Vec<u8>,
}

impl Received {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("a JSON body")
    }
}

#[derive(Clone, Default)]
struct Log {
    received: Arc<Mutex<Vec<Received>>>,
    /// Paths answering 500 once, then 200.
    failing_once: Arc<Mutex<BTreeSet<String>>>,
}

/// Records every POST by path, and answers 500 to the first one on a path set to fail once.
async fn receiver(failing_once: &[&str]) -> (String, Log) {
    let log = Log::default();
    log.failing_once
        .lock()
        .unwrap()
        .extend(failing_once.iter().map(|path| (*path).to_owned()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .fallback(
            |axum::extract::State(log): axum::extract::State<Log>,
             uri: axum::http::Uri,
             headers: axum::http::HeaderMap,
             body: axum::body::Bytes| async move {
                let path = uri.path().to_owned();
                log.received.lock().unwrap().push(Received {
                    path: path.clone(),
                    id: headers["webhook-id"].to_str().unwrap().to_owned(),
                    signature: headers["webhook-signature"].to_str().unwrap().to_owned(),
                    body: body.to_vec(),
                });
                if log.failing_once.lock().unwrap().remove(&path) {
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR
                } else {
                    axum::http::StatusCode::OK
                }
            },
        )
        .with_state(log.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, log)
}

fn on(log: &Log, path: &str) -> Vec<Received> {
    log.received
        .lock()
        .unwrap()
        .iter()
        .filter(|received| received.path == path)
        .cloned()
        .collect()
}

async fn endpoint(f: &Fixture, url: String, api_version: Option<&str>) -> WebhookEndpoint {
    f.setup
        .store
        .create_webhook_endpoint(
            TENANT_ID,
            WebhookEndpointNew {
                url,
                description: None,
                event_types: vec![],
                max_in_flight: 10,
                headers: vec![],
                rate_limit_per_sec: None,
                api_version: api_version.map(str::to_owned),
            },
        )
        .await
        .unwrap()
        .0
}

/// Every outbox event queued so far turned into messages and deliveries.
async fn fan_out(f: &Fixture) {
    for _ in 0..30 {
        processors::run_once_webhook_out(
            Arc::new(f.setup.store.clone()),
            Arc::new(f.setup.services.clone()),
        )
        .await;
    }
}

/// Every delivery due sent, retries made due at once.
async fn deliver(f: &Fixture) {
    let worker = WebhookDeliveryWorker::new(
        Arc::new(f.setup.store.clone()),
        &meteroid::process::egress(&f.setup.config.webhook_out),
    )
    .unwrap();
    for _ in 0..50 {
        let mut conn = f.setup.store.pool.get().await.unwrap();
        diesel::sql_query(
            "UPDATE webhook_out_delivery SET next_attempt_at = now() WHERE status = 'PENDING'",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        if worker.run_tick().await.claimed == 0 {
            return;
        }
    }
}

/// What the v2 GET of the resource an event's `data` names answers now.
async fn read_back(f: &Fixture, event_type: &str, data: &Value) -> Option<Value> {
    let group = event_type.split('.').next().unwrap();
    let id = match data["id"].as_str() {
        Some(id) => id,
        // A notification: no resource to read back.
        None => return None,
    };
    let path = match group {
        "customer" => format!("/api/v2/customers/{id}"),
        "subscription" => format!("/api/v2/subscriptions/{id}"),
        "invoice" => format!("/api/v2/invoices/{id}"),
        "credit_note" => format!("/api/v2/credit-notes/{id}"),
        "product" => format!("/api/v2/products/{id}"),
        "addon" => format!("/api/v2/add-ons/{id}"),
        "metric" => format!("/api/v2/metrics/{id}"),
        "coupon" => format!("/api/v2/coupons/{id}"),
        "plan" => format!(
            "/api/v2/plans/{id}?version={}",
            data["version"]["number"].as_i64().unwrap()
        ),
        _ => return None,
    };
    let read = f.get(&path).await;
    assert_eq!(read.status, StatusCode::OK, "{path}: {:#}", read.body);
    Some(read.body)
}

/// Rules: ADR-0013 §15 (payload versions)
#[tokio::test]
async fn test_rest_v2_webhook_payloads_follow_the_endpoint_version() {
    let f = fixture().await;
    let (base, log) = receiver(&["/v2"]).await;
    let v2 = endpoint(&f, format!("{base}/v2"), Some(V2)).await;
    let v1 = endpoint(&f, format!("{base}/v1"), None).await;
    endpoint(&f, format!("{base}/pinned"), Some("2026-12-01")).await;

    // The catalog (products, an add-on, a coupon, plans), a customer, a subscription and its
    // invoice, a one-off invoice issued and paid and a credit note on it.
    seed(&f).await;
    let customer = f
        .post_once(
            "/api/v2/customers",
            "hooks-customer",
            json!({"name": "Hooks Co", "alias": "hooks-co", "currency": "EUR"}),
        )
        .await;
    assert_eq!(customer.status, StatusCode::CREATED, "{:#}", customer.body);
    let subscription = f
        .post_once(
            "/api/v2/subscriptions",
            "hooks-subscription",
            json!({
                "customer": "ext:hooks-co",
                "plan": "studio",
                "option": "monthly",
                "quantities": {"users": "3"},
                "levels": {"api": "1000"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(
        subscription.status,
        StatusCode::CREATED,
        "{:#}",
        subscription.body
    );
    let invoice = f
        .post_once(
            "/api/v2/invoices",
            "hooks-invoice",
            json!({
                "customer": "ext:hooks-co",
                "currency": "EUR",
                "status": "finalized",
                "lines": [{"name": "Onboarding", "quantity": "2", "unit_price": "150"}]
            }),
        )
        .await;
    assert_eq!(invoice.status, StatusCode::CREATED, "{:#}", invoice.body);
    let paid = f
        .post(
            &format!(
                "/api/v2/invoices/{}/mark-paid",
                invoice.body["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await;
    assert_eq!(paid.status, StatusCode::OK, "{:#}", paid.body);
    let note = f
        .post_once(
            "/api/v2/credit-notes",
            "hooks-note",
            json!({
                "invoice": invoice.body["id"],
                "lines": [{"line": invoice.body["lines"][0]["id"], "quantity": "1"}],
                "reason": "One session cancelled"
            }),
        )
        .await;
    assert_eq!(note.status, StatusCode::CREATED, "{:#}", note.body);

    fan_out(&f).await;
    deliver(&f).await;

    // Every endpoint got every event; the v2 one twice its first (a 500, then the retry).
    let to_v2 = on(&log, "/v2");
    let to_v1 = on(&log, "/v1");
    let to_pinned = on(&log, "/pinned");
    let ids = |received: &[Received]| -> BTreeSet<String> {
        received.iter().map(|r| r.id.clone()).collect()
    };
    assert!(!to_v1.is_empty());
    assert_eq!(ids(&to_v2), ids(&to_v1), "no event is missing at v2");
    assert_eq!(ids(&to_pinned), ids(&to_v1));
    assert_eq!(to_v2.len(), to_v1.len() + 1);

    // A retry sends the very bytes sent first.
    let first = &to_v2[0];
    let again: Vec<&Received> = to_v2.iter().filter(|r| r.id == first.id).collect();
    assert_eq!(
        again.len(),
        2,
        "the first delivery failed, then was retried"
    );
    assert_eq!(again[0].body, again[1].body);

    // v2's envelopes: `data` is what the resource's v2 GET answers.
    let mut types = BTreeSet::new();
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for received in &to_v2 {
        let payload = received.json();
        let keys: BTreeSet<&str> = payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["id", "type", "created_at", "api_version", "data"]),
            "{payload:#}"
        );
        assert_eq!(payload["id"], received.id.as_str());
        assert_eq!(payload["api_version"], V2);
        let event_type = payload["type"].as_str().unwrap().to_owned();
        if let Some(read) = read_back(&f, &event_type, &payload["data"]).await {
            assert_eq!(payload["data"], read, "{event_type}");
        }
        types.insert(event_type);
        by_id.insert(received.id.clone(), payload);
    }
    for expected in [
        "customer.created",
        "subscription.created",
        "invoice.created",
        "invoice.finalized",
        "invoice.paid",
        "credit_note.created",
        "product.created",
        "plan.published",
    ] {
        assert!(types.contains(expected), "{expected} in {types:?}");
    }
    let created = by_id
        .values()
        .find(|payload| payload["type"] == "customer.created")
        .unwrap();
    assert_eq!(created["data"]["id"], customer.body["id"]);
    assert_eq!(created["data"]["alias"], "hooks-co");

    // No version, or one before v2's payloads: v1's payloads, as stored for the event, byte for
    // byte.
    for received in to_v1.iter().chain(&to_pinned) {
        let stored = f
            .setup
            .store
            .get_webhook_message(EventId::from_str(&received.id).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(received.body, serde_json::to_vec(&stored.payload).unwrap());
        let payload = received.json();
        assert!(payload.get("api_version").is_none(), "{payload:#}");
        assert!(payload.get("timestamp").is_some(), "{payload:#}");
    }
    let v1_created = to_v1
        .iter()
        .map(Received::json)
        .find(|payload| payload["type"] == "customer.created")
        .unwrap();
    assert_eq!(v1_created["customer_id"], customer.body["id"]);
    assert_eq!(v1_created["name"], "Hooks Co");
    let pinned_bodies: BTreeMap<String, Vec<u8>> = to_pinned
        .iter()
        .map(|r| (r.id.clone(), r.body.clone()))
        .collect();
    for received in &to_v1 {
        assert_eq!(pinned_bodies[&received.id], received.body);
    }

    // A resend sends the bytes the delivery was made with.
    let v2_deliveries = f
        .setup
        .store
        .list_webhook_deliveries_before(TENANT_ID, v2.id, None, None, 100)
        .await
        .unwrap()
        .unwrap();
    let resent = v2_deliveries
        .iter()
        .find(|item| item.event_type == "customer.created")
        .unwrap();
    f.setup
        .store
        .resend_webhook_delivery(TENANT_ID, resent.delivery.id)
        .await
        .unwrap();

    // The v1 endpoint moves to v2's payloads: from its next event on.
    let upgraded = f
        .patch(
            &format!("/api/v2/webhooks/endpoints/{}", v1.id),
            json!({"api_version": V2}),
        )
        .await;
    assert_eq!(upgraded.status, StatusCode::OK, "{:#}", upgraded.body);
    assert_eq!(upgraded.body["api_version"], V2);
    let v1_deliveries = f
        .setup
        .store
        .list_webhook_deliveries_before(TENANT_ID, v1.id, None, None, 100)
        .await
        .unwrap()
        .unwrap();
    let old = v1_deliveries
        .iter()
        .find(|item| item.event_type == "customer.created")
        .unwrap();
    f.setup
        .store
        .resend_webhook_delivery(TENANT_ID, old.delivery.id)
        .await
        .unwrap();
    let later = f
        .post_once(
            "/api/v2/customers",
            "hooks-later",
            json!({"name": "Later Co", "currency": "EUR"}),
        )
        .await;
    assert_eq!(later.status, StatusCode::CREATED, "{:#}", later.body);
    let seen = log.received.lock().unwrap().len();
    fan_out(&f).await;
    deliver(&f).await;
    let since: Vec<Received> = log.received.lock().unwrap()[seen..].to_vec();
    let v2_resent = since
        .iter()
        .find(|r| r.path == "/v2" && r.id == resent.delivery.message_id.to_string())
        .expect("the resend was sent");
    assert_eq!(
        v2_resent.body,
        to_v2.iter().find(|r| r.id == v2_resent.id).unwrap().body,
        "a resend sends the first bytes"
    );
    let v1_resent = since
        .iter()
        .find(|r| r.path == "/v1" && r.id == old.delivery.message_id.to_string())
        .expect("the old delivery was resent");
    assert_eq!(
        v1_resent.body,
        to_v1.iter().find(|r| r.id == v1_resent.id).unwrap().body,
        "a delivery made before the upgrade keeps its shape"
    );
    let next = since
        .iter()
        .filter(|r| r.path == "/v1" && r.id != old.delivery.message_id.to_string())
        .map(Received::json)
        .find(|payload| payload["type"] == "customer.created")
        .expect("the next event reached the upgraded endpoint");
    assert_eq!(next["api_version"], V2);
    assert_eq!(next["data"]["id"], later.body["id"]);
    assert_eq!(
        next["data"],
        f.get(&format!(
            "/api/v2/customers/{}",
            later.body["id"].as_str().unwrap()
        ))
        .await
        .body
    );
    let pinned_next = since
        .iter()
        .filter(|r| r.path == "/pinned")
        .map(Received::json)
        .find(|payload| payload["type"] == "customer.created")
        .unwrap();
    assert!(
        pinned_next.get("api_version").is_none(),
        "a pinned endpoint never changes shape: {pinned_next:#}"
    );
}

/// Rules: ADR-0013 §15 (an endpoint created from the dashboard)
#[tokio::test]
async fn test_dashboard_endpoints_take_the_latest_payload_version() {
    let f = fixture().await;
    let auth = meteroid_it::svc_auth::login(f.setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        f.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let mut webhooks = clients.webhooks_out.clone();

    let created = webhooks
        .create_webhook_endpoint(grpc::CreateWebhookEndpointRequest {
            url: "https://hooks.example/dashboard".to_owned(),
            description: None,
            event_types: vec![],
            headers: vec![],
            rate_limit_per_sec: None,
        })
        .await
        .unwrap()
        .into_inner()
        .endpoint
        .unwrap();
    assert_eq!(created.api_version.as_deref(), Some(V2));
    assert!(!created.v1_payloads);

    // One on v1's payloads (created through v1 or before versions) is upgraded in one action.
    let legacy = endpoint(&f, "https://hooks.example/legacy".to_owned(), None).await;
    let listed = webhooks
        .get_webhook_endpoint(grpc::GetWebhookEndpointRequest {
            id: legacy.id.as_proto(),
        })
        .await
        .unwrap()
        .into_inner()
        .endpoint
        .unwrap();
    assert!(listed.api_version.is_none());
    assert!(listed.v1_payloads);
    let upgraded = webhooks
        .upgrade_webhook_endpoint_api_version(grpc::UpgradeWebhookEndpointApiVersionRequest {
            id: legacy.id.as_proto(),
        })
        .await
        .unwrap()
        .into_inner()
        .endpoint
        .unwrap();
    assert_eq!(upgraded.api_version.as_deref(), Some(V2));
    assert!(!upgraded.v1_payloads);
    let stored = f
        .setup
        .store
        .get_webhook_endpoint(
            TENANT_ID,
            WebhookEndpointId::from_proto(&upgraded.id).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stored.api_version.as_deref(), Some(V2));

    // Its test event is a v2 envelope.
    let test = webhooks
        .send_test_webhook(grpc::SendTestWebhookRequest {
            endpoint_id: legacy.id.as_proto(),
            event_type: None,
        })
        .await
        .unwrap()
        .into_inner()
        .delivery
        .unwrap();
    let details = f
        .setup
        .store
        .get_webhook_delivery(
            TENANT_ID,
            common_domain::ids::WebhookDeliveryId::from_proto(&test.id).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(details.delivery.api_version.as_deref(), Some(V2));
    assert_eq!(details.message.payload["type"], "webhook.test");
    assert_eq!(details.message.payload["api_version"], V2);
    assert_eq!(
        details.message.payload["data"]["endpoint"],
        legacy.id.as_proto()
    );
}

/// What `ListWebhookEventTypes` lists for one payload shape, by event type.
async fn catalog(
    clients: &AllClients,
    v2_payloads: bool,
) -> BTreeMap<String, grpc::WebhookEventType> {
    clients
        .webhooks_out
        .clone()
        .list_webhook_event_types(grpc::ListWebhookEventTypesRequest { v2_payloads })
        .await
        .unwrap()
        .into_inner()
        .event_types
        .into_iter()
        .map(|event_type| (event_type.name.clone(), event_type))
        .collect()
}

/// Rules: ADR-0013 §15 (the dashboard's catalog and examples follow the endpoint's payloads)
#[tokio::test]
async fn test_rest_v2_webhook_catalog_follows_the_payload_version() {
    let f = fixture().await;
    let auth = meteroid_it::svc_auth::login(f.setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        f.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let mut webhooks = clients.webhooks_out.clone();

    // v1's catalog, as it always was: flat payloads under `timestamp`.
    let v1 = catalog(&clients, false).await;
    let paid = &v1["invoice.paid"];
    let example: Value = serde_json::from_str(&paid.example_json).unwrap();
    assert!(example.get("timestamp").is_some(), "{example:#}");
    assert!(example.get("api_version").is_none());
    assert_eq!(paid.fields[2].path, "timestamp");

    // v2's: the envelope, `data` the resource as v2 reads it, an example per event type.
    let v2 = catalog(&clients, true).await;
    assert!(
        !v2.contains_key("entitlement.too_many_groups"),
        "retired in v2"
    );
    for (name, event_type) in &v2 {
        let example: Value = serde_json::from_str(&event_type.example_json).unwrap();
        let keys: BTreeSet<&str> = example
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["id", "type", "created_at", "api_version", "data"]),
            "{name}"
        );
        assert_eq!(example["type"], name.as_str());
        assert_eq!(example["api_version"], V2);
        assert!(example["data"].is_object(), "{name}: {example:#}");
        let envelope: Vec<&str> = event_type.fields[..5]
            .iter()
            .map(|field| field.path.as_str())
            .collect();
        assert_eq!(
            envelope,
            ["id", "type", "created_at", "api_version", "data"],
            "{name}"
        );
        assert_eq!(event_type.fields[1].r#type, format!("\"{name}\""));
    }
    let customer: Value = serde_json::from_str(&v2["customer.created"].example_json).unwrap();
    assert!(customer["data"]["id"].as_str().unwrap().starts_with("cus_"));
    let funded = &v2["wallet.funded"];
    let example: Value = serde_json::from_str(&funded.example_json).unwrap();
    for resource in ["wallet", "ledger_entry", "credit_grant"] {
        assert!(example["data"][resource].is_object(), "{example:#}");
    }
    let paths: BTreeSet<&str> = funded.fields.iter().map(|f| f.path.as_str()).collect();
    for path in [
        "data.wallet.credit_on_account",
        "data.ledger_entry.balance_after",
        "data.credit_grant.remaining",
    ] {
        assert!(paths.contains(path), "{path} in {paths:?}");
    }

    // "Send example" to an endpoint on v2's payloads: that example in the v2 envelope at its
    // version, signed and delivered as a real event; v1's beside it in the message.
    let (base, log) = receiver(&[]).await;
    let v2_endpoint = endpoint(&f, format!("{base}/v2"), Some(V2)).await;
    let sent = webhooks
        .send_test_webhook(grpc::SendTestWebhookRequest {
            endpoint_id: v2_endpoint.id.as_proto(),
            event_type: Some("wallet.funded".to_owned()),
        })
        .await
        .unwrap()
        .into_inner()
        .delivery
        .unwrap();
    let details = f
        .setup
        .store
        .get_webhook_delivery(
            TENANT_ID,
            common_domain::ids::WebhookDeliveryId::from_proto(&sent.id).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(details.delivery.api_version.as_deref(), Some(V2));
    assert_eq!(details.message.event_type, "wallet.funded");
    assert_eq!(details.message.payload["api_version"], V2);
    let v1_beside = f
        .setup
        .store
        .get_webhook_message(details.delivery.message_id, None)
        .await
        .unwrap();
    assert!(
        v1_beside.payload.get("timestamp").is_some(),
        "{:#}",
        v1_beside.payload
    );
    deliver(&f).await;
    let received = on(&log, "/v2");
    assert_eq!(received.len(), 1);
    let payload = received[0].json();
    assert_eq!(payload["id"], received[0].id.as_str());
    assert_eq!(payload["type"], "wallet.funded");
    assert_eq!(payload["api_version"], V2);
    assert_eq!(payload["data"], example["data"]);
    assert!(
        received[0].signature.starts_with("v1,"),
        "{}",
        received[0].signature
    );
    let unknown = webhooks
        .send_test_webhook(grpc::SendTestWebhookRequest {
            endpoint_id: v2_endpoint.id.as_proto(),
            event_type: Some("entitlement.too_many_groups".to_owned()),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), tonic::Code::InvalidArgument);

    // An endpoint on v1's payloads is sent v1's example, as before.
    let v1_endpoint = endpoint(&f, format!("{base}/v1"), None).await;
    webhooks
        .send_test_webhook(grpc::SendTestWebhookRequest {
            endpoint_id: v1_endpoint.id.as_proto(),
            event_type: Some("invoice.paid".to_owned()),
        })
        .await
        .unwrap();
    deliver(&f).await;
    let received = on(&log, "/v1");
    assert_eq!(received.len(), 1);
    let payload = received[0].json();
    assert_eq!(payload["type"], "invoice.paid");
    assert!(payload.get("api_version").is_none(), "{payload:#}");
    assert!(payload.get("timestamp").is_some());
}

/// The v2 GET of a customer's wallet, credit grant or ledger line `data` names.
async fn credit_read(f: &Fixture, customer: &str, what: &str, value: &Value) -> Value {
    let path = match what {
        "wallet" => format!(
            "/api/v2/customers/{customer}/wallets/{}",
            value["currency"].as_str().unwrap()
        ),
        "credit_grant" => format!(
            "/api/v2/customers/{customer}/credit-grants/{}",
            value["id"].as_str().unwrap()
        ),
        "ledger_entry" => {
            let ledger = f
                .get(&format!(
                    "/api/v2/customers/{customer}/credit-ledger?limit=100"
                ))
                .await;
            assert_eq!(ledger.status, StatusCode::OK, "{:#}", ledger.body);
            return ledger.body["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == value["id"])
                .cloned()
                .unwrap_or_else(|| panic!("{value:#} is in the ledger"));
        }
        _ => unreachable!(),
    };
    let read = f.get(&path).await;
    assert_eq!(read.status, StatusCode::OK, "{path}: {:#}", read.body);
    read.body
}

/// Rules: ADR-0013 §15 (a notification carries the resources it is about)
#[tokio::test]
async fn test_rest_v2_webhook_credit_payloads_carry_their_resources() {
    let f = fixture().await;
    seed(&f).await;
    let (base, log) = receiver(&[]).await;
    endpoint(&f, format!("{base}/v2"), Some(V2)).await;

    let customer = f
        .post_once(
            "/api/v2/customers",
            "credit-hooks-customer",
            json!({"name": "Credit Hooks Co", "alias": "credit-hooks", "currency": "EUR"}),
        )
        .await;
    assert_eq!(customer.status, StatusCode::CREATED, "{:#}", customer.body);
    let customer_id = customer.body["id"].as_str().unwrap().to_owned();
    let grants = "/api/v2/customers/ext:credit-hooks/credit-grants";

    // Credit granted (`wallet.funded`), part of it removed by hand (`wallet.debited`), another
    // grant lapsing (`wallet.funded`, then `credit_grant.expired`).
    let kept = f
        .post_once(
            grants,
            "credit-hooks-kept",
            json!({"amount": 5_000, "type": "promotional", "reason": "Welcome"}),
        )
        .await;
    assert_eq!(kept.status, StatusCode::CREATED, "{:#}", kept.body);
    let kept_id = CreditGrantId::from_str(kept.body["id"].as_str().unwrap()).unwrap();
    let customer_uuid = CustomerId::from_str(&customer_id).unwrap();
    f.setup
        .store
        .debit_customer_credit_grant(DebitCustomerCreditGrant {
            tenant_id: TENANT_ID,
            customer_id: customer_uuid,
            grant_id: kept_id,
            amount_minor: Some(2_000),
            idempotency_key: uuid::Uuid::now_v7(),
            reason: "Granted twice".to_owned(),
            actor_id: None,
        })
        .await
        .unwrap();
    let lapsing = f
        .post_once(
            grants,
            "credit-hooks-lapsing",
            json!({
                "amount": 3_000,
                "type": "promotional",
                "expires_at": (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            }),
        )
        .await;
    assert_eq!(lapsing.status, StatusCode::CREATED, "{:#}", lapsing.body);
    let lapsing_id = CreditGrantId::from_str(lapsing.body["id"].as_str().unwrap()).unwrap();
    expire_grant(&f.setup.store, lapsing_id).await;
    assert_eq!(f.setup.store.expire_customer_credit(200).await.unwrap(), 1);

    // An automatic top-up held back by its cap, as the usage controls notify it.
    let wallet_id = f
        .setup
        .store
        .customer_credit_wallets(TENANT_ID, customer_uuid)
        .await
        .unwrap()
        .into_iter()
        .find(|wallet| wallet.currency == "EUR")
        .unwrap()
        .id;
    let now = common_clock::now();
    let capped =
        OutboxEvent::WalletAutoTopUpCapReached(Box::new(WalletAutoTopUpCapReachedEvent::new(
            TENANT_ID,
            customer_uuid,
            wallet_id,
            "EUR".to_owned(),
            TopUpCapPeriod::Daily,
            10_000,
            5_000,
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(23),
        )));

    // A paid checkout whose subscription could not be created: its session cancelled, the
    // payment kept as credit.
    let session = f
        .post_once(
            "/api/v2/checkout-sessions",
            "credit-hooks-checkout",
            json!({
                "customer": "ext:spotify",
                "plan": "studio",
                "option": "annual",
                "quantities": {"users": "4"}
            }),
        )
        .await;
    assert_eq!(session.status, StatusCode::CREATED, "{:#}", session.body);
    let session_id = session.body["id"].as_str().unwrap().to_owned();
    let cancelled = f
        .post(
            &format!("/api/v2/checkout-sessions/{session_id}/cancel"),
            json!({}),
        )
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:#}", cancelled.body);
    let stored = f
        .setup
        .store
        .get_checkout_session(TENANT_ID, CheckoutSessionId::from_str(&session_id).unwrap())
        .await
        .unwrap();
    let kept_as_credit =
        OutboxEvent::checkout_payment_kept_as_credit(CheckoutPaymentKeptAsCreditEvent {
            id: EventId::new(),
            tenant_id: TENANT_ID,
            customer_id: stored.customer_id,
            checkout_session_id: stored.id,
            payment_transaction_id: PaymentTransactionId::new(),
            plan_version_id: stored.plan_version_id,
            resource_id: stored.resource_id,
            amount: 12_000,
            currency: "EUR".to_owned(),
            reason: "The subscription could not be created".to_owned(),
        });
    let mut conn = f.setup.store.pool.get().await.unwrap();
    publish_outbox_events_tx(&mut conn, vec![capped, kept_as_credit])
        .await
        .unwrap();
    drop(conn);

    fan_out(&f).await;
    deliver(&f).await;

    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for received in on(&log, "/v2") {
        let payload = received.json();
        let event_type = payload["type"].as_str().unwrap().to_owned();
        let data = &payload["data"];
        let resources: &[&str] = match event_type.as_str() {
            "wallet.funded" | "wallet.debited" => &["wallet", "ledger_entry", "credit_grant"],
            "credit_grant.expired" => &["ledger_entry", "credit_grant"],
            "wallet.auto_top_up_cap_reached" => &["wallet"],
            "checkout.payment_kept_as_credit" => {
                let read = f
                    .get(&format!("/api/v2/checkout-sessions/{session_id}"))
                    .await;
                assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
                assert_eq!(data["checkout_session"], read.body, "{event_type}");
                assert_eq!(data["checkout_session"]["status"], "cancelled");
                assert!(data["checkout_session"].get("url").is_none());
                assert_eq!(data["amount"], 12_000);
                *seen.entry(event_type).or_default() += 1;
                continue;
            }
            _ => continue,
        };
        assert_eq!(data["customer"]["id"], customer_id.as_str(), "{event_type}");
        for resource in resources {
            let read = credit_read(&f, &customer_id, resource, &data[*resource]).await;
            assert_eq!(data[*resource], read, "{event_type}: {resource}");
        }
        match event_type.as_str() {
            "wallet.debited" => {
                assert_eq!(data["reason"], "Granted twice");
                assert_eq!(data["ledger_entry"]["amount"], -2_000);
                assert_eq!(data["credit_grant"]["id"], kept.body["id"]);
            }
            "credit_grant.expired" => {
                assert_eq!(data["credit_grant"]["id"], lapsing.body["id"]);
                assert_eq!(data["credit_grant"]["status"], "expired");
                assert_eq!(data["expired_amount"], 3_000);
                assert_eq!(data["reason"], "expired");
            }
            "wallet.auto_top_up_cap_reached" => {
                assert_eq!(data["wallet"]["currency"], "EUR");
                assert_eq!(data["cap"], 10_000);
                assert_eq!(data["period"], "daily");
            }
            _ => {}
        }
        *seen.entry(event_type).or_default() += 1;
    }
    assert_eq!(seen.get("wallet.funded"), Some(&2), "{seen:?}");
    for event_type in [
        "wallet.debited",
        "credit_grant.expired",
        "wallet.auto_top_up_cap_reached",
        "checkout.payment_kept_as_credit",
    ] {
        assert_eq!(seen.get(event_type), Some(&1), "{event_type} in {seen:?}");
    }
}

/// Rules: ADR-0013 §15 (a retention notice carries its subscription)
#[tokio::test]
async fn test_rest_v2_webhook_retention_payloads_carry_their_subscription() {
    let f = fixture().await;
    seed(&f).await;
    let (base, log) = receiver(&[]).await;
    endpoint(&f, format!("{base}/v2"), Some(V2)).await;
    endpoint(&f, format!("{base}/v1"), None).await;

    let customer = f
        .post_once(
            "/api/v2/customers",
            "retention-hooks-customer",
            json!({"name": "Retention Hooks Co", "alias": "retention-hooks", "currency": "EUR"}),
        )
        .await;
    assert_eq!(customer.status, StatusCode::CREATED, "{:#}", customer.body);
    let subscription = f
        .post_once(
            "/api/v2/subscriptions",
            "retention-hooks-subscription",
            json!({
                "customer": "ext:retention-hooks",
                "plan": "studio",
                "option": "annual",
                "quantities": {"users": "3"},
                "invoicing": {"charge_automatically": false}
            }),
        )
        .await;
    assert_eq!(
        subscription.status,
        StatusCode::CREATED,
        "{:#}",
        subscription.body
    );
    let customer_id = CustomerId::from_str(customer.body["id"].as_str().unwrap()).unwrap();
    let subscription_id =
        SubscriptionId::from_str(subscription.body["id"].as_str().unwrap()).unwrap();

    // The portal's cancel flow ending on an offer, and a renewal's checkpoint, as the retention
    // module records them.
    let now = common_clock::now();
    let session_id = RetentionSessionId::new();
    let renewal_date = now.date_naive() + chrono::Duration::days(60);
    let completed = OutboxEvent::retention_session_completed(RetentionSessionEvent {
        id: RetentionSessionEvent::event_id(session_id),
        tenant_id: TENANT_ID,
        session_id,
        customer_id,
        subscription_id,
        outcome: RetentionOutcome::ContactRequested,
        reason: Some("Too expensive".to_owned()),
        comment: None,
        offer_kind: Some(RetentionOfferKind::PlanChange),
        offer_title: Some("Move to the monthly plan".to_owned()),
        effective_date: Some(renewal_date),
        completed_at: now,
    });
    let upcoming = OutboxEvent::retention_renewal_upcoming(RenewalUpcomingEvent {
        id: RenewalUpcomingEvent::event_id(
            subscription_id,
            renewal_date,
            60,
            RenewalMode::Renewing,
        ),
        tenant_id: TENANT_ID,
        customer_id,
        subscription_id,
        renewal_date,
        mode: RenewalMode::Renewing,
        days_before: 60,
        plan_name: "Studio".to_owned(),
        currency: "EUR".to_owned(),
        mrr_cents: 12_300,
        notified_at: now,
    });
    let mut conn = f.setup.store.pool.get().await.unwrap();
    publish_outbox_events_tx(&mut conn, vec![completed, upcoming])
        .await
        .unwrap();
    drop(conn);

    fan_out(&f).await;
    deliver(&f).await;

    let read = f
        .get(&format!("/api/v2/subscriptions/{subscription_id}"))
        .await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    let mut seen = BTreeSet::new();
    for received in on(&log, "/v2") {
        let payload = received.json();
        let event_type = payload["type"].as_str().unwrap().to_owned();
        if !event_type.starts_with("retention.") {
            continue;
        }
        let data = &payload["data"];
        assert_eq!(payload["api_version"], V2);
        assert_eq!(data["customer"]["id"], customer.body["id"], "{event_type}");
        assert_eq!(data["subscription"], read.body, "{event_type}");
        match event_type.as_str() {
            "retention.session_completed" => {
                assert_eq!(data["session"], session_id.to_string());
                assert_eq!(data["outcome"], "contact_requested");
                assert_eq!(data["reason"], "Too expensive");
                assert!(data.get("comment").is_none(), "{data:#}");
                assert_eq!(
                    data["offer"],
                    json!({"kind": "plan_change", "title": "Move to the monthly plan"})
                );
                assert_eq!(data["effective_date"], renewal_date.to_string());
                assert!(data["completed_at"].is_string());
            }
            "retention.renewal_upcoming" => {
                assert_eq!(data["renewal_date"], renewal_date.to_string());
                assert_eq!(data["mode"], "renewing");
                assert_eq!(data["days_before"], 60);
                assert_eq!(data["mrr"], json!({"amount": 12_300, "currency": "EUR"}));
                assert!(data.get("plan_name").is_none(), "{data:#}");
                assert!(data["notified_at"].is_string());
            }
            _ => unreachable!("{event_type}"),
        }
        seen.insert(event_type);
    }
    assert_eq!(
        seen,
        BTreeSet::from([
            "retention.renewal_upcoming".to_owned(),
            "retention.session_completed".to_owned()
        ])
    );

    // An endpoint on v1's payloads keeps v1's flat shape for them, as stored for the event.
    let mut seen = BTreeSet::new();
    for received in on(&log, "/v1") {
        let payload = received.json();
        let event_type = payload["type"].as_str().unwrap().to_owned();
        if !event_type.starts_with("retention.") {
            continue;
        }
        let stored = f
            .setup
            .store
            .get_webhook_message(EventId::from_str(&received.id).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(received.body, serde_json::to_vec(&stored.payload).unwrap());
        assert!(payload.get("data").is_none(), "{payload:#}");
        assert!(payload.get("timestamp").is_some(), "{payload:#}");
        assert_eq!(payload["customer_id"], customer.body["id"]);
        assert_eq!(payload["subscription_id"], subscription.body["id"]);
        match event_type.as_str() {
            "retention.session_completed" => {
                assert_eq!(payload["session_id"], session_id.to_string());
                assert_eq!(payload["outcome"], "CONTACT_REQUESTED");
                assert_eq!(payload["offer_kind"], "PLAN_CHANGE");
                assert_eq!(payload["offer_title"], "Move to the monthly plan");
            }
            "retention.renewal_upcoming" => {
                assert_eq!(payload["mode"], "RENEWING");
                assert_eq!(payload["plan_name"], "Studio");
                assert_eq!(payload["mrr_cents"], 12_300);
                assert_eq!(payload["currency"], "EUR");
            }
            _ => unreachable!("{event_type}"),
        }
        seen.insert(event_type);
    }
    assert_eq!(seen.len(), 2, "{seen:?}");
}
