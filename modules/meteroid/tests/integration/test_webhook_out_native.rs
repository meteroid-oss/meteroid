//! End-to-end coverage of the native outbound webhook engine: outbox event -> pgmq
//! fan-out -> stored message + deliveries -> signed HTTP POST -> attempt/endpoint state.
//! Signatures are recomputed with an independent HMAC so signing drift fails the test.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, EventId, WebhookDeliveryId, WebhookEndpointId};
use diesel_async::RunQueryDsl;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use crate::data::ids::TENANT_ID;
use crate::harness::{TestEnv, test_env_minimal};
use crate::helpers::outbox::publish_entitlement_limit_reached;
use crate::meteroid_it;
use meteroid::services::egress::{Egress, EgressPurpose};
use meteroid::services::webhooks_out::{
    MAX_IN_FLIGHT_PRODUCTION, MAX_IN_FLIGHT_SANDBOX, tenant_max_in_flight,
};
use meteroid::workers::pgmq::processors;
use meteroid::workers::webhook_delivery::WebhookDeliveryWorker;
use meteroid_store::domain::enums::CustomerType;
use meteroid_store::domain::enums::{WebhookDeliveryStatus, WebhookEndpointDisabledReason};
use meteroid_store::domain::misc::PaginationRequest;
use meteroid_store::domain::webhooks_out::{
    WebhookDelivery, WebhookDeliveryListItem, WebhookEndpoint, WebhookEndpointNew,
};
use meteroid_store::domain::{CustomerNew, CustomerTaxStatus};
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::WebhookOutInterface;

// ---------------------------------------------------------------------------
// Local HTTP receiver
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ReceivedRequest {
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl ReceivedRequest {
    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .unwrap_or_else(|| panic!("missing header {name} in {:?}", self.headers))
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("body is valid json")
    }
}

#[derive(Clone)]
struct ReceiverState {
    received: Arc<Mutex<Vec<ReceivedRequest>>>,
    statuses: Arc<Mutex<VecDeque<u16>>>,
}

/// Records every POST it gets and answers with the next queued status, then 200.
struct Receiver {
    url: String,
    state: ReceiverState,
}

impl Receiver {
    async fn spawn(statuses: Vec<u16>) -> Self {
        let state = ReceiverState {
            received: Arc::new(Mutex::new(Vec::new())),
            statuses: Arc::new(Mutex::new(statuses.into())),
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind receiver");
        let addr = listener.local_addr().expect("receiver addr");

        let app = axum::Router::new()
            .fallback(record_request)
            .with_state(state.clone());

        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            url: format!("http://{addr}/hook"),
            state,
        }
    }

    fn requests(&self) -> Vec<ReceivedRequest> {
        self.state.received.lock().unwrap().clone()
    }
}

async fn record_request(
    axum::extract::State(state): axum::extract::State<ReceiverState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    let headers = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();

    state.received.lock().unwrap().push(ReceivedRequest {
        headers,
        body: body.to_vec(),
    });

    let status = state.statuses.lock().unwrap().pop_front().unwrap_or(200);
    axum::http::StatusCode::from_u16(status).expect("valid status")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `v1,<base64(HMAC_SHA256(key, "{id}.{ts}.{body}"))>` recomputed from scratch.
fn expected_signature(
    secret: &SecretString,
    message_id: &str,
    timestamp: &str,
    body: &[u8],
) -> String {
    let raw = secret.expose_secret();
    let key = STANDARD
        .decode(raw.strip_prefix("whsec_").unwrap_or(raw))
        .expect("secret is base64");

    let mut signed = format!("{message_id}.{timestamp}.").into_bytes();
    signed.extend_from_slice(body);

    format!(
        "v1,{}",
        STANDARD.encode(hmac_sha256::HMAC::mac(&signed, &key))
    )
}

async fn create_endpoint(
    env: &TestEnv,
    url: &str,
    event_types: Vec<String>,
) -> (WebhookEndpoint, SecretString) {
    env.store()
        .create_webhook_endpoint(
            TENANT_ID,
            WebhookEndpointNew {
                url: url.to_string(),
                description: Some("integration test endpoint".to_string()),
                event_types,
                max_in_flight: default_max_in_flight(env).await,
                headers: vec![],
                rate_limit_per_sec: None,
            },
        )
        .await
        .expect("create webhook endpoint")
}

/// The cap the gRPC and REST create paths would resolve for the fixture tenant.
async fn default_max_in_flight(env: &TestEnv) -> i32 {
    tenant_max_in_flight(env.store(), TENANT_ID)
        .await
        .expect("resolve max in flight")
}

/// Creating a customer publishes exactly one `customer.created` outbox event.
async fn create_customer(env: &TestEnv, name: &str) {
    env.store()
        .insert_customer(
            Actor::System,
            CustomerNew {
                name: name.to_string(),
                customer_type: CustomerType::Company,
                first_name: None,
                last_name: None,
                alias: None,
                billing_email: None,
                invoicing_emails: vec![],
                phone: None,
                currency: "EUR".to_string(),
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                force_created_date: None,
                tax_status: CustomerTaxStatus::Taxable,
                exemption_reason: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: vec![],
                connected_account_id: None,
                custom_properties: serde_json::Value::Null,
                preferred_locales: vec![],
            },
            TENANT_ID,
        )
        .await
        .expect("insert customer");
}

async fn fan_out(env: &TestEnv) {
    processors::run_once_webhook_out(Arc::new(env.store().clone())).await;
}

fn worker(env: &TestEnv) -> WebhookDeliveryWorker {
    WebhookDeliveryWorker::new(Arc::new(env.store().clone()), &core_egress(env))
        .expect("build delivery worker")
}

fn core_egress(env: &TestEnv) -> Egress {
    meteroid::process::egress(&env.setup.config.webhook_out)
}

async fn deliveries(env: &TestEnv, endpoint_id: WebhookEndpointId) -> Vec<WebhookDeliveryListItem> {
    env.store()
        .list_webhook_deliveries(
            TENANT_ID,
            endpoint_id,
            None,
            PaginationRequest {
                per_page: Some(50),
                page: 0,
            },
        )
        .await
        .expect("list deliveries")
        .items
}

async fn message_count(env: &TestEnv) -> i64 {
    let mut conn = env.conn().await;
    diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT count(*) FROM webhook_out_message")
        .get_result(&mut conn)
        .await
        .expect("count messages")
}

/// Makes every pending retry due right now, so the next tick claims it without waiting.
async fn force_due(env: &TestEnv) {
    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE webhook_out_delivery SET next_attempt_at = now() WHERE status = 'PENDING'",
    )
    .execute(&mut conn)
    .await
    .expect("force deliveries due");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full happy path: outbox event -> stored message + delivery -> signed POST -> SUCCEEDED.
#[rstest]
#[tokio::test]
async fn native_webhook_is_fanned_out_delivered_and_signed(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![]).await;

    let (endpoint, secret) = create_endpoint(&env, &receiver.url, vec![]).await;

    create_customer(&env, "Webhook Signing Co").await;
    fan_out(&env).await;

    assert_eq!(message_count(&env).await, 1, "one stored webhook message");

    let pending = deliveries(&env, endpoint.id).await;
    assert_eq!(pending.len(), 1, "one delivery for the only endpoint");
    assert_eq!(pending[0].event_type, "customer.created");
    assert_eq!(pending[0].delivery.status, WebhookDeliveryStatus::Pending);

    let message_id = pending[0].delivery.message_id;

    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.claimed, 1);
    assert_eq!(stats.succeeded, 1);
    assert_eq!(stats.failed, 0);

    let requests = receiver.requests();
    assert_eq!(requests.len(), 1, "exactly one POST");
    let request = &requests[0];

    let envelope = request.json();
    assert_eq!(envelope["type"], "customer.created");
    assert_eq!(envelope["id"], message_id.to_string());
    assert_eq!(envelope["name"], "Webhook Signing Co");

    assert_eq!(request.header("webhook-id"), message_id.to_string());
    assert!(
        !request.headers.contains_key("svix-id") && !request.headers.contains_key("svix-signature"),
        "an endpoint created natively only gets the Standard Webhooks headers"
    );
    assert_eq!(request.header("content-type"), "application/json");
    assert_eq!(request.header("user-agent"), "Meteroid-Webhooks/1.0");

    assert_eq!(
        request.header("webhook-signature"),
        expected_signature(
            &secret,
            &message_id.to_string(),
            request.header("webhook-timestamp"),
            &request.body,
        ),
        "signature verifies against an independently computed HMAC"
    );

    let delivery = &deliveries(&env, endpoint.id).await[0].delivery;
    assert_eq!(delivery.status, WebhookDeliveryStatus::Succeeded);
    assert_eq!(delivery.attempt_count, 1);
    assert_eq!(delivery.last_response_status, Some(200));
    assert!(delivery.completed_at.is_some());
    assert!(delivery.next_attempt_at.is_none());

    let details = env
        .store()
        .get_webhook_delivery(TENANT_ID, delivery.id)
        .await
        .expect("delivery details");
    assert_eq!(details.attempts.len(), 1);
    assert!(details.attempts[0].succeeded);
    assert_eq!(details.attempts[0].attempt_number, 1);
    assert_eq!(details.attempts[0].response_status, Some(200));
    assert_eq!(details.message.event_type, "customer.created");

    let endpoint = env
        .store()
        .get_webhook_endpoint(TENANT_ID, endpoint.id)
        .await
        .expect("reload endpoint");
    assert_eq!(endpoint.consecutive_failures, 0);
    assert!(endpoint.last_success_at.is_some());
    assert!(!endpoint.disabled);
}

/// A 500 reschedules the delivery ~5s out; once due it is retried and succeeds.
#[rstest]
#[tokio::test]
async fn a_failed_attempt_is_rescheduled_and_retried(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![500]).await;

    let (endpoint, _secret) = create_endpoint(&env, &receiver.url, vec![]).await;

    create_customer(&env, "Retry Co").await;
    fan_out(&env).await;

    let before = chrono::Utc::now().naive_utc();
    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.claimed, 1);
    assert_eq!(stats.failed, 1);

    let delivery = deliveries(&env, endpoint.id).await[0].delivery.clone();
    assert_eq!(delivery.status, WebhookDeliveryStatus::Pending);
    assert_eq!(delivery.attempt_count, 1);
    assert_eq!(delivery.last_response_status, Some(500));
    assert!(delivery.completed_at.is_none());

    // First retry delay is 5s ± 10% jitter, measured from the moment of the attempt.
    let next = delivery.next_attempt_at.expect("retry is scheduled");
    let delay = (next - before).num_milliseconds();
    assert!(
        (4_000..=8_000).contains(&delay),
        "first retry is roughly 5s out, got {delay}ms"
    );

    let failing = env
        .store()
        .get_webhook_endpoint(TENANT_ID, endpoint.id)
        .await
        .expect("reload endpoint");
    assert_eq!(failing.consecutive_failures, 1);
    assert!(failing.last_failure_at.is_some());

    force_due(&env).await;

    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.claimed, 1);
    assert_eq!(stats.succeeded, 1);

    let delivery = deliveries(&env, endpoint.id).await[0].delivery.clone();
    assert_eq!(delivery.status, WebhookDeliveryStatus::Succeeded);
    assert_eq!(delivery.attempt_count, 2);

    let details = env
        .store()
        .get_webhook_delivery(TENANT_ID, delivery.id)
        .await
        .expect("delivery details");
    assert_eq!(details.attempts.len(), 2);
    assert_eq!(receiver.requests().len(), 2);

    let recovered = env
        .store()
        .get_webhook_endpoint(TENANT_ID, endpoint.id)
        .await
        .expect("reload endpoint");
    assert_eq!(recovered.consecutive_failures, 0);
    assert!(recovered.last_success_at.is_some());
}

/// An endpoint subscribed to another event type produces no message and no delivery.
#[rstest]
#[tokio::test]
async fn unsubscribed_event_types_are_not_fanned_out(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![]).await;

    let (endpoint, _secret) =
        create_endpoint(&env, &receiver.url, vec!["invoice.finalized".to_string()]).await;

    create_customer(&env, "Filtered Co").await;
    fan_out(&env).await;

    assert_eq!(message_count(&env).await, 0, "no message is stored");
    assert!(deliveries(&env, endpoint.id).await.is_empty());

    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.claimed, 0);
    assert!(receiver.requests().is_empty());
}

/// A beta event type is not in the published spec: an "all events" endpoint never gets it,
/// while an endpoint that lists it does. Published types still reach "all events".
#[rstest]
#[tokio::test]
async fn beta_event_types_reach_only_endpoints_that_list_them(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![]).await;

    let (all_events, _) = create_endpoint(&env, &receiver.url, vec![]).await;
    let (subscribed, _) = create_endpoint(
        &env,
        &receiver.url,
        vec!["entitlement.limit_reached".to_string()],
    )
    .await;

    publish_entitlement_limit_reached(&mut env.conn().await, TENANT_ID).await;
    create_customer(&env, "Beta Filter Co").await;
    fan_out(&env).await;

    let to_all: Vec<_> = deliveries(&env, all_events.id)
        .await
        .into_iter()
        .map(|d| d.event_type)
        .collect();
    assert_eq!(to_all, vec!["customer.created".to_string()]);

    let to_subscribed: Vec<_> = deliveries(&env, subscribed.id)
        .await
        .into_iter()
        .map(|d| d.event_type)
        .collect();
    assert_eq!(to_subscribed, vec!["entitlement.limit_reached".to_string()]);
}

/// HTTP 410 fails the delivery outright and takes the endpoint out of service.
#[rstest]
#[tokio::test]
async fn gone_fails_the_delivery_and_disables_the_endpoint(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![410]).await;

    let (endpoint, _secret) = create_endpoint(&env, &receiver.url, vec![]).await;

    create_customer(&env, "Gone Co").await;
    fan_out(&env).await;

    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.claimed, 1);
    assert_eq!(stats.failed, 1);

    let delivery = deliveries(&env, endpoint.id).await[0].delivery.clone();
    assert_eq!(delivery.status, WebhookDeliveryStatus::Failed);
    assert_eq!(delivery.last_response_status, Some(410));
    assert!(delivery.next_attempt_at.is_none());
    assert!(delivery.completed_at.is_some());

    let endpoint = env
        .store()
        .get_webhook_endpoint(TENANT_ID, endpoint.id)
        .await
        .expect("reload endpoint");
    assert!(endpoint.disabled);
    assert_eq!(
        endpoint.disabled_reason,
        Some(WebhookEndpointDisabledReason::Gone)
    );
    assert!(endpoint.disabled_at.is_some());
}

/// During the rotation grace window both secrets sign the request, new one first.
#[rstest]
#[tokio::test]
async fn a_rotated_endpoint_signs_with_both_secrets(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![]).await;

    let (endpoint, previous_secret) = create_endpoint(&env, &receiver.url, vec![]).await;

    let new_secret = env
        .store()
        .rotate_webhook_endpoint_secret(TENANT_ID, endpoint.id)
        .await
        .expect("rotate secret");
    assert_ne!(
        new_secret.expose_secret(),
        previous_secret.expose_secret(),
        "rotation issues a different secret"
    );

    create_customer(&env, "Rotation Co").await;
    fan_out(&env).await;

    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.succeeded, 1);

    let requests = receiver.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];

    let header = request.header("webhook-signature");
    let signatures: Vec<&str> = header.split(' ').collect();
    assert_eq!(
        signatures.len(),
        2,
        "both secrets sign during the grace window"
    );

    let message_id = deliveries(&env, endpoint.id).await[0]
        .delivery
        .message_id
        .to_string();
    let timestamp = request.header("webhook-timestamp");

    assert_eq!(
        signatures[0],
        expected_signature(&new_secret, &message_id, timestamp, &request.body),
        "the first signature is the one produced by the new secret"
    );
    assert_eq!(
        signatures[1],
        expected_signature(&previous_secret, &message_id, timestamp, &request.body),
        "the rotated-out secret keeps verifying"
    );
}

/// Marks the delivery terminal the way an exhausted retry budget would.
async fn exhaust_retries(env: &TestEnv) {
    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE webhook_out_delivery SET status = 'FAILED', next_attempt_at = NULL, completed_at = now()",
    )
    .execute(&mut conn)
    .await
    .expect("mark delivery failed");
}

/// Puts the delivery under a live lease, as a worker that is mid-POST would.
async fn lease_delivery(env: &TestEnv) {
    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE webhook_out_delivery SET status = 'IN_FLIGHT', lease_expires_at = now() + interval '60 seconds'",
    )
    .execute(&mut conn)
    .await
    .expect("lease delivery");
}

/// Resending a terminal delivery grants a fresh budget without restarting the attempt log.
#[rstest]
#[tokio::test]
async fn a_resend_continues_the_attempt_numbering(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![500]).await;

    let (endpoint, _secret) = create_endpoint(&env, &receiver.url, vec![]).await;

    create_customer(&env, "Resend Co").await;
    fan_out(&env).await;

    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.failed, 1);

    let delivery = deliveries(&env, endpoint.id).await[0].delivery.clone();
    exhaust_retries(&env).await;

    let resent = env
        .store()
        .resend_webhook_delivery(TENANT_ID, delivery.id)
        .await
        .expect("resend");
    assert_eq!(resent.status, WebhookDeliveryStatus::Pending);
    assert_eq!(resent.attempt_count, 0, "the resend gets a fresh budget");

    force_due(&env).await;
    let stats = worker(&env).run_tick().await;
    assert_eq!(stats.succeeded, 1);

    let details = env
        .store()
        .get_webhook_delivery(TENANT_ID, delivery.id)
        .await
        .expect("delivery details");
    let numbers: Vec<i32> = details
        .attempts
        .iter()
        .map(|attempt| attempt.attempt_number)
        .collect();
    assert_eq!(numbers, vec![1, 2], "one ordered timeline, no duplicates");
    assert!(details.attempts[1].succeeded);
}

/// Resending a delivery that a worker holds would drop its lease and duplicate the POST.
#[rstest]
#[tokio::test]
async fn a_resend_is_refused_while_the_delivery_is_in_flight(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![]).await;

    let (endpoint, _secret) = create_endpoint(&env, &receiver.url, vec![]).await;

    create_customer(&env, "In Flight Co").await;
    fan_out(&env).await;

    let delivery = deliveries(&env, endpoint.id).await[0].delivery.clone();
    lease_delivery(&env).await;

    assert!(
        env.store()
            .resend_webhook_delivery(TENANT_ID, delivery.id)
            .await
            .is_err(),
        "a leased delivery must not be rescheduled under its worker"
    );
}

// ---------------------------------------------------------------------------
// Per-endpoint in-flight cap
// ---------------------------------------------------------------------------

async fn set_max_in_flight(env: &TestEnv, endpoint_id: WebhookEndpointId, max_in_flight: i32) {
    let mut conn = env.conn().await;
    diesel::sql_query("UPDATE webhook_out_endpoint SET max_in_flight = $1 WHERE id = $2")
        .bind::<diesel::sql_types::Integer, _>(max_in_flight)
        .bind::<diesel::sql_types::Uuid, _>(*endpoint_id)
        .execute(&mut conn)
        .await
        .expect("set max_in_flight");
}

/// Gives an endpoint's due deliveries distinct past due times and returns their ids
/// oldest-first, so what the claim picks is unambiguous.
async fn stagger_due_times(
    env: &TestEnv,
    endpoint_id: WebhookEndpointId,
) -> Vec<WebhookDeliveryId> {
    let mut ids: Vec<WebhookDeliveryId> = deliveries(env, endpoint_id)
        .await
        .into_iter()
        .map(|item| item.delivery.id)
        .collect();
    ids.sort_by_key(|id| **id);

    let mut conn = env.conn().await;
    for (index, id) in ids.iter().enumerate() {
        let seconds_ago = ((ids.len() - index) * 10) as i32;
        diesel::sql_query(
            "UPDATE webhook_out_delivery SET next_attempt_at = now() - ($1::int * interval '1 second') WHERE id = $2",
        )
        .bind::<diesel::sql_types::Integer, _>(seconds_ago)
        .bind::<diesel::sql_types::Uuid, _>(**id)
        .execute(&mut conn)
        .await
        .expect("stagger due times");
    }

    ids
}

async fn claim(env: &TestEnv) -> Vec<WebhookDelivery> {
    env.store()
        .claim_due_webhook_deliveries(10, 90)
        .await
        .expect("claim due deliveries")
}

/// A cap of 1 keeps an endpoint serialised: one delivery per claim, and nothing more
/// while that one still holds a live lease.
#[rstest]
#[tokio::test]
async fn a_cap_of_one_claims_a_single_delivery(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;

    let (endpoint, _secret) = create_endpoint(&env, "https://capped.example/hook", vec![]).await;
    set_max_in_flight(&env, endpoint.id, 1).await;

    for name in ["Cap One A", "Cap One B", "Cap One C"] {
        create_customer(&env, name).await;
    }
    fan_out(&env).await;

    let ordered = stagger_due_times(&env, endpoint.id).await;
    assert_eq!(ordered.len(), 3, "three due deliveries for one endpoint");

    let claimed = claim(&env).await;
    assert_eq!(
        claimed.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![ordered[0]],
        "only the oldest due delivery is claimed"
    );

    assert!(
        claim(&env).await.is_empty(),
        "the live lease leaves no free slot"
    );
}

/// A cap of 2 lets two deliveries of the same endpoint run at once, still oldest first.
#[rstest]
#[tokio::test]
async fn a_cap_of_two_claims_the_two_oldest(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;

    let (endpoint, _secret) = create_endpoint(&env, "https://burst.example/hook", vec![]).await;
    set_max_in_flight(&env, endpoint.id, 2).await;

    for name in ["Cap Two A", "Cap Two B", "Cap Two C"] {
        create_customer(&env, name).await;
    }
    fan_out(&env).await;

    let ordered = stagger_due_times(&env, endpoint.id).await;

    // `UPDATE ... RETURNING` has no row order of its own, so compare by due time.
    let mut claimed = claim(&env).await;
    claimed.sort_by_key(|delivery| delivery.next_attempt_at);
    assert_eq!(
        claimed.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![ordered[0], ordered[1]],
        "the two oldest due deliveries fill the endpoint's two slots"
    );

    assert!(claim(&env).await.is_empty(), "the cap is not exceeded");
}

/// The cap is per endpoint, so a busy endpoint never crowds another one out.
#[rstest]
#[tokio::test]
async fn capped_endpoints_are_claimed_side_by_side(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;

    let (first, _secret) = create_endpoint(&env, "https://first.example/hook", vec![]).await;
    let (second, _secret) = create_endpoint(&env, "https://second.example/hook", vec![]).await;
    set_max_in_flight(&env, first.id, 1).await;
    set_max_in_flight(&env, second.id, 1).await;

    for name in ["Fair A", "Fair B"] {
        create_customer(&env, name).await;
    }
    fan_out(&env).await;

    let first_ordered = stagger_due_times(&env, first.id).await;
    let second_ordered = stagger_due_times(&env, second.id).await;
    assert_eq!(first_ordered.len(), 2);
    assert_eq!(second_ordered.len(), 2);

    let claimed = claim(&env).await;
    assert_eq!(claimed.len(), 2, "one delivery per endpoint");

    let mut per_endpoint: Vec<(WebhookEndpointId, WebhookDeliveryId)> =
        claimed.iter().map(|d| (d.endpoint_id, d.id)).collect();
    per_endpoint.sort_by_key(|(endpoint_id, _)| **endpoint_id);

    let mut expected = vec![(first.id, first_ordered[0]), (second.id, second_ordered[0])];
    expected.sort_by_key(|(endpoint_id, _)| **endpoint_id);

    assert_eq!(per_endpoint, expected);
}

/// The cap a new endpoint starts on comes from its tenant's environment. The fixture
/// tenant is a development one, so it must get the sandbox value, not the production one.
#[rstest]
#[tokio::test]
async fn a_new_endpoint_takes_its_tenant_environment_cap(#[future] test_env_minimal: TestEnv) {
    use meteroid_grpc::meteroid::api::webhooks::out::v1::CreateWebhookEndpointRequest;

    let env = test_env_minimal.await;
    assert_ne!(
        MAX_IN_FLIGHT_SANDBOX, MAX_IN_FLIGHT_PRODUCTION,
        "the two caps must differ for this assertion to mean anything"
    );

    let auth = meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        env.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );

    let created = clients
        .webhooks_out
        .clone()
        .create_webhook_endpoint(CreateWebhookEndpointRequest {
            url: "https://grpc-created.example/hook".to_string(),
            description: None,
            event_types: vec![],
            headers: vec![],
            rate_limit_per_sec: None,
        })
        .await
        .expect("create endpoint over grpc")
        .into_inner();

    assert_eq!(
        created.endpoint.expect("endpoint").max_in_flight,
        MAX_IN_FLIGHT_SANDBOX
    );
}

// ---------------------------------------------------------------------------
// Claims as retry budget
// ---------------------------------------------------------------------------

/// Ends every live lease, as if the worker holding it had died.
async fn expire_leases(env: &TestEnv) {
    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE webhook_out_delivery SET lease_expires_at = (now() AT TIME ZONE 'UTC') - interval '1 second' WHERE status = 'IN_FLIGHT'",
    )
    .execute(&mut conn)
    .await
    .expect("expire leases");
}

async fn delivery_row(env: &TestEnv, endpoint_id: WebhookEndpointId) -> WebhookDelivery {
    deliveries(env, endpoint_id).await[0].delivery.clone()
}

/// A claim whose result never makes it to the database (a dead worker, a failed write,
/// an undecryptable endpoint) still spends one attempt once its lease expires.
#[rstest]
#[tokio::test]
async fn a_lost_record_still_consumes_budget(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let (endpoint, _secret) = create_endpoint(&env, "https://lost.example/hook", vec![]).await;

    create_customer(&env, "Lost Record Co").await;
    fan_out(&env).await;

    let claimed = claim(&env).await;
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        claimed[0].attempt_count, 1,
        "the claim itself spends the attempt"
    );

    expire_leases(&env).await;
    assert_eq!(
        env.store()
            .reclaim_expired_webhook_deliveries()
            .await
            .expect("reclaim"),
        1
    );

    let reclaimed = delivery_row(&env, endpoint.id).await;
    assert_eq!(reclaimed.status, WebhookDeliveryStatus::Pending);
    assert_eq!(reclaimed.attempt_count, 1);
    assert!(
        reclaimed.next_attempt_at.expect("rescheduled") > chrono::Utc::now().naive_utc(),
        "retried on the backoff schedule, not immediately"
    );
    assert!(reclaimed.last_error.is_some());

    force_due(&env).await;
    let again = claim(&env).await;
    assert_eq!(again[0].attempt_count, 2);
}

/// An expired lease on the last attempt ends the delivery instead of re-queueing it.
#[rstest]
#[tokio::test]
async fn an_exhausted_delivery_whose_lease_expires_fails(#[future] test_env_minimal: TestEnv) {
    use meteroid_store::services::webhooks::out::schedule::MAX_ATTEMPTS;

    let env = test_env_minimal.await;
    let (endpoint, _secret) = create_endpoint(&env, "https://exhausted.example/hook", vec![]).await;

    create_customer(&env, "Exhausted Co").await;
    fan_out(&env).await;

    let mut conn = env.conn().await;
    diesel::sql_query("UPDATE webhook_out_delivery SET attempt_count = $1")
        .bind::<diesel::sql_types::Integer, _>(MAX_ATTEMPTS - 1)
        .execute(&mut conn)
        .await
        .expect("spend the budget");

    let claimed = claim(&env).await;
    assert_eq!(claimed[0].attempt_count, MAX_ATTEMPTS);

    expire_leases(&env).await;
    env.store()
        .reclaim_expired_webhook_deliveries()
        .await
        .expect("reclaim");

    let failed = delivery_row(&env, endpoint.id).await;
    assert_eq!(failed.status, WebhookDeliveryStatus::Failed);
    assert!(failed.next_attempt_at.is_none());
    assert!(failed.completed_at.is_some());
    assert!(claim(&env).await.is_empty(), "never claimed again");
}

/// A worker that outlived its lease must not overwrite the claim that replaced it.
#[rstest]
#[tokio::test]
async fn a_late_finisher_cannot_overwrite_a_newer_claim(#[future] test_env_minimal: TestEnv) {
    use meteroid_store::domain::webhooks_out::WebhookAttemptOutcome;

    let env = test_env_minimal.await;
    let (endpoint, _secret) = create_endpoint(&env, "https://late.example/hook", vec![]).await;

    create_customer(&env, "Late Finisher Co").await;
    fan_out(&env).await;

    let first = claim(&env).await.remove(0);
    expire_leases(&env).await;
    env.store()
        .reclaim_expired_webhook_deliveries()
        .await
        .expect("reclaim");
    force_due(&env).await;
    let second = claim(&env).await.remove(0);
    assert_eq!(second.id, first.id);

    let succeeded = |claim: &WebhookDelivery| WebhookAttemptOutcome {
        delivery_id: claim.id,
        endpoint_id: claim.endpoint_id,
        attempt_number: claim.attempt_count,
        lease_expires_at: claim.lease_expires_at.expect("leased"),
        duration_ms: 10,
        response_status: Some(200),
        response_body: Some("ok".to_string()),
        error: None,
        succeeded: true,
        next_attempt_at: None,
        final_status: WebhookDeliveryStatus::Succeeded,
        disable_endpoint: None,
        endpoint_wide_failure: false,
        pause_endpoint_until: None,
        refund_attempt: false,
    };

    env.store()
        .record_webhook_attempt(succeeded(&first))
        .await
        .expect("late record");

    let current = delivery_row(&env, endpoint.id).await;
    assert_eq!(current.status, WebhookDeliveryStatus::InFlight);
    assert_eq!(current.attempt_count, second.attempt_count);
    assert_eq!(current.lease_expires_at, second.lease_expires_at);

    env.store()
        .record_webhook_attempt(succeeded(&second))
        .await
        .expect("current record");
    assert_eq!(
        delivery_row(&env, endpoint.id).await.status,
        WebhookDeliveryStatus::Succeeded
    );
}

/// The claim is bounded by endpoints, so a deep backlog on one endpoint cannot hide the
/// single due delivery of another.
#[rstest]
#[tokio::test]
async fn a_backlog_on_one_endpoint_does_not_hide_another(#[future] test_env_minimal: TestEnv) {
    use meteroid_store::domain::webhooks_out::WebhookOutMessageNew;

    let env = test_env_minimal.await;
    let (busy, _secret) = create_endpoint(&env, "https://busy.example/hook", vec![]).await;
    let (quiet, _secret) = create_endpoint(&env, "https://quiet.example/hook", vec![]).await;
    set_max_in_flight(&env, busy.id, 2).await;
    set_max_in_flight(&env, quiet.id, 1).await;

    let insert = |endpoint_id: WebhookEndpointId, index: usize| {
        let store = env.store().clone();
        async move {
            store
                .insert_webhook_message_with_deliveries(
                    WebhookOutMessageNew {
                        id: EventId::new(),
                        tenant_id: TENANT_ID,
                        event_type: "customer.created".to_string(),
                        payload: serde_json::json!({ "index": index }),
                    },
                    &[endpoint_id],
                )
                .await
                .expect("insert delivery");
        }
    };
    for index in 0..40 {
        insert(busy.id, index).await;
    }
    insert(quiet.id, 40).await;

    let busy_ordered = stagger_due_times(&env, busy.id).await;
    assert_eq!(busy_ordered.len(), 40);

    let claimed = claim(&env).await;
    let mut busy_claimed: Vec<WebhookDeliveryId> = claimed
        .iter()
        .filter(|d| d.endpoint_id == busy.id)
        .map(|d| d.id)
        .collect();
    busy_claimed.sort_by_key(|id| **id);
    let mut expected_busy = busy_ordered[..2].to_vec();
    expected_busy.sort_by_key(|id| **id);

    assert_eq!(
        busy_claimed, expected_busy,
        "the busy endpoint's two oldest"
    );
    assert_eq!(
        claimed.iter().filter(|d| d.endpoint_id == quiet.id).count(),
        1,
        "the quiet endpoint is claimed too"
    );
}

// ---------------------------------------------------------------------------
// Endpoint input caps
// ---------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn a_tenant_is_capped_at_fifty_endpoints(#[future] test_env_minimal: TestEnv) {
    use meteroid_store::errors::StoreError;
    use meteroid_store::repositories::webhooks_out::MAX_ENDPOINTS_PER_TENANT;

    let env = test_env_minimal.await;
    for index in 0..MAX_ENDPOINTS_PER_TENANT {
        create_endpoint(&env, &format!("https://cap{index}.example/hook"), vec![]).await;
    }

    let max_in_flight = default_max_in_flight(&env).await;
    let refused = env
        .store()
        .create_webhook_endpoint(
            TENANT_ID,
            WebhookEndpointNew {
                url: "https://one-too-many.example/hook".to_string(),
                description: None,
                event_types: vec![],
                max_in_flight,
                headers: vec![],
                rate_limit_per_sec: None,
            },
        )
        .await
        .expect_err("the 51st endpoint is refused");
    assert!(matches!(
        refused.current_context(),
        StoreError::FailedPrecondition(_)
    ));

    let first = env
        .store()
        .list_webhook_endpoints(TENANT_ID)
        .await
        .expect("list")[0]
        .id;
    env.store()
        .archive_webhook_endpoint(TENANT_ID, first)
        .await
        .expect("archive");
    create_endpoint(&env, "https://after-archive.example/hook", vec![]).await;
}

#[rstest]
#[tokio::test]
async fn an_update_can_clear_the_description(#[future] test_env_minimal: TestEnv) {
    use meteroid_store::domain::webhooks_out::WebhookEndpointPatch;

    let env = test_env_minimal.await;
    let (endpoint, _secret) = create_endpoint(&env, "https://describe.example/hook", vec![]).await;
    assert!(endpoint.description.is_some());

    let patch = |description| WebhookEndpointPatch {
        id: endpoint.id,
        description,
        ..WebhookEndpointPatch::default()
    };

    let kept = env
        .store()
        .update_webhook_endpoint(TENANT_ID, patch(None))
        .await
        .expect("no-op update");
    assert_eq!(
        kept.description, endpoint.description,
        "absent leaves it alone"
    );

    let cleared = env
        .store()
        .update_webhook_endpoint(TENANT_ID, patch(Some(None)))
        .await
        .expect("clearing update");
    assert!(cleared.description.is_none());
}

// ---------------------------------------------------------------------------
// Unreachable endpoints and rate limits
// ---------------------------------------------------------------------------

async fn insert_deliveries(env: &TestEnv, endpoint_id: WebhookEndpointId, count: usize) {
    for index in 0..count {
        env.store()
            .insert_webhook_message_with_deliveries(
                meteroid_store::domain::webhooks_out::WebhookOutMessageNew {
                    id: EventId::new(),
                    tenant_id: TENANT_ID,
                    event_type: "customer.created".to_string(),
                    payload: serde_json::json!({ "index": index }),
                },
                &[endpoint_id],
            )
            .await
            .expect("insert message");
    }
}

async fn endpoint(env: &TestEnv, id: WebhookEndpointId) -> WebhookEndpoint {
    env.store()
        .get_webhook_endpoint(TENANT_ID, id)
        .await
        .expect("get endpoint")
}

async fn end_pause(env: &TestEnv, id: WebhookEndpointId) {
    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE webhook_out_endpoint SET paused_until = now() - interval '1 second' WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(*id)
    .execute(&mut conn)
    .await
    .expect("end pause");
}

/// A dead receiver gets a few attempts, then the endpoint is paused and probed one
/// delivery at a time; the probe that trips the pause does not spend retry budget.
#[rstest]
#[tokio::test]
async fn an_unreachable_endpoint_is_paused_then_probed(#[future] test_env_minimal: TestEnv) {
    use meteroid_store::services::webhooks::out::schedule::PROBE_THRESHOLD;

    let env = test_env_minimal.await;
    // Nothing listens on port 1: every attempt is refused, i.e. no response at all.
    let (dead, _secret) = create_endpoint(&env, "http://127.0.0.1:1/hook", vec![]).await;
    set_max_in_flight(&env, dead.id, 1).await;
    insert_deliveries(&env, dead.id, 3).await;

    for _ in 0..PROBE_THRESHOLD {
        force_due(&env).await;
        worker(&env).run_tick().await;
    }

    let paused = endpoint(&env, dead.id).await;
    assert_eq!(paused.unreachable_streak, PROBE_THRESHOLD);
    assert!(paused.paused_until.is_some(), "paused after the threshold");
    assert!(paused.failing_since.is_some());

    let spent: i32 = deliveries(&env, dead.id)
        .await
        .iter()
        .map(|d| d.delivery.attempt_count)
        .sum();
    assert_eq!(
        spent,
        PROBE_THRESHOLD - 1,
        "the tripping attempt was refunded"
    );

    let listed = deliveries(&env, dead.id).await;
    let made: i64 = listed.iter().map(|d| d.attempts).sum();
    assert_eq!(
        made, PROBE_THRESHOLD as i64,
        "the attempt count includes the refunded probe"
    );
    let error = listed
        .iter()
        .find_map(|d| d.delivery.last_error.clone())
        .expect("a failed attempt recorded its error");
    assert!(
        error.starts_with("could not connect: ") && error.to_lowercase().contains("refused"),
        "the error names its cause: {error}"
    );

    let retrying = listed.iter().filter(|d| d.attempts > 0).count() as i64;
    let days = env
        .store()
        .webhook_endpoint_daily_stats(
            TENANT_ID,
            dead.id,
            chrono::Utc::now().naive_utc() - chrono::Duration::days(1),
        )
        .await
        .expect("daily stats");
    assert_eq!(days.len(), 1);
    assert_eq!(
        (days[0].succeeded, days[0].failed, days[0].retrying),
        (0, 0, retrying),
        "deliveries that failed and are still retried are counted"
    );

    force_due(&env).await;
    assert!(claim(&env).await.is_empty(), "nothing is sent while paused");

    end_pause(&env, dead.id).await;
    set_max_in_flight(&env, dead.id, 8).await;
    force_due(&env).await;
    assert_eq!(claim(&env).await.len(), 1, "probed one delivery at a time");
}

/// Behind a dead egress proxy nothing reaches any endpoint: deliveries wait, and no
/// endpoint is paused, disabled or charged an attempt for our outage.
#[rstest]
#[tokio::test]
async fn a_dead_egress_proxy_defers_without_blaming_the_endpoint(
    #[future] test_env_minimal: TestEnv,
) {
    use meteroid_store::services::webhooks::out::schedule::PROBE_THRESHOLD;

    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![]).await;
    let (healthy, _secret) = create_endpoint(&env, &receiver.url, vec![]).await;
    insert_deliveries(&env, healthy.id, 1).await;

    let behind_dead_proxy = WebhookDeliveryWorker::new(
        Arc::new(env.store().clone()),
        &meteroid::process::egress(&meteroid::config::WebhookOutConfig {
            egress_proxy_url: Some("http://127.0.0.1:1".to_string()),
            ..env.setup.config.webhook_out.clone()
        }),
    )
    .expect("build delivery worker");

    for _ in 0..=PROBE_THRESHOLD {
        force_due(&env).await;
        behind_dead_proxy.run_tick().await;
    }

    let untouched = endpoint(&env, healthy.id).await;
    assert_eq!(untouched.unreachable_streak, 0);
    assert_eq!(untouched.consecutive_failures, 0);
    assert!(untouched.failing_since.is_none());
    assert!(untouched.paused_until.is_none());
    assert!(untouched.disabled_reason.is_none());

    let listed = &deliveries(&env, healthy.id).await[0];
    assert_eq!(listed.delivery.status, WebhookDeliveryStatus::Pending);
    assert_eq!(listed.delivery.attempt_count, 0, "no budget spent");
    assert_eq!(listed.attempts, 0, "no attempt logged against the endpoint");
    assert!(
        listed
            .delivery
            .last_error
            .as_deref()
            .is_some_and(|e| e.starts_with("delivery delayed")),
        "{:?}",
        listed.delivery.last_error
    );
    assert!(receiver.requests().is_empty());

    force_due(&env).await;
    worker(&env).run_tick().await;
    assert_eq!(
        receiver.requests().len(),
        1,
        "delivered once egress is back"
    );
}

/// A receiver that answers, even with an error, is reachable: only that event retries.
#[rstest]
#[tokio::test]
async fn an_event_specific_rejection_never_pauses(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![400; 10]).await;
    let (rejecting, _secret) = create_endpoint(&env, &receiver.url, vec![]).await;
    set_max_in_flight(&env, rejecting.id, 1).await;
    insert_deliveries(&env, rejecting.id, 1).await;

    for _ in 0..6 {
        force_due(&env).await;
        worker(&env).run_tick().await;
    }

    let after = endpoint(&env, rejecting.id).await;
    assert_eq!(after.consecutive_failures, 6);
    assert_eq!(after.unreachable_streak, 0);
    assert!(after.paused_until.is_none());
}

/// A rate-limited endpoint gets no more claims per second than its limit.
#[rstest]
#[tokio::test]
async fn a_rate_limited_endpoint_is_claimed_up_to_its_rate(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let (limited, _secret) = env
        .store()
        .create_webhook_endpoint(
            TENANT_ID,
            WebhookEndpointNew {
                url: "https://limited.example/hook".to_string(),
                description: None,
                event_types: vec![],
                max_in_flight: 8,
                headers: vec![],
                rate_limit_per_sec: Some(2),
            },
        )
        .await
        .expect("create endpoint");
    insert_deliveries(&env, limited.id, 5).await;

    assert_eq!(claim(&env).await.len(), 2, "two per second");
    assert!(claim(&env).await.is_empty(), "the second is used up");
}

/// "Retry now" lifts the pause immediately.
#[rstest]
#[tokio::test]
async fn resuming_lifts_the_pause(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let (paused, _secret) = create_endpoint(&env, "https://paused.example/hook", vec![]).await;

    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE webhook_out_endpoint SET paused_until = now() + interval '1 hour', unreachable_streak = 9 WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(*paused.id)
    .execute(&mut conn)
    .await
    .expect("pause");

    let resumed = env
        .store()
        .resume_webhook_endpoint(TENANT_ID, paused.id)
        .await
        .expect("resume");
    assert!(resumed.paused_until.is_none());
    assert_eq!(resumed.unreachable_streak, 9, "still probed one at a time");
}

/// The success-rate numbers count only finished deliveries, per day and per endpoint.
#[rstest]
#[tokio::test]
async fn stats_count_finished_deliveries(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let (endpoint_a, _) = create_endpoint(&env, "https://a.example/hook", vec![]).await;
    insert_deliveries(&env, endpoint_a.id, 5).await;

    let mut conn = env.conn().await;
    diesel::sql_query(
        "UPDATE webhook_out_delivery SET status = CASE
            WHEN id IN (SELECT id FROM webhook_out_delivery ORDER BY id LIMIT 3) THEN 'SUCCEEDED'
            WHEN id IN (SELECT id FROM webhook_out_delivery ORDER BY id OFFSET 3 LIMIT 1) THEN 'FAILED'
            ELSE status END::\"WebhookDeliveryStatusEnum\"",
    )
    .execute(&mut conn)
    .await
    .expect("finish some deliveries");

    let since = chrono::Utc::now().naive_utc() - chrono::Duration::days(7);
    let per_endpoint = env
        .store()
        .webhook_endpoints_stats(TENANT_ID, since)
        .await
        .expect("endpoint stats");
    assert_eq!(per_endpoint.len(), 1);
    assert_eq!((per_endpoint[0].succeeded, per_endpoint[0].failed), (3, 1));

    let days = env
        .store()
        .webhook_endpoint_daily_stats(TENANT_ID, endpoint_a.id, since)
        .await
        .expect("daily stats");
    assert_eq!(days.len(), 1, "all created today");
    assert_eq!(
        (days[0].succeeded, days[0].failed),
        (3, 1),
        "the pending one is not counted"
    );
}

/// A receiver answering a redirect never sends the delivery elsewhere: following it would
/// let any public URL bounce the request onto an internal address past the egress guard.
#[rstest]
#[tokio::test]
async fn redirects_are_never_followed(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let target = Receiver::spawn(vec![]).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirector");
    let redirector_url = format!("http://{}/hook", listener.local_addr().expect("addr"));
    let location = target.url.clone();
    let app = axum::Router::new().fallback(move || {
        let location = location.clone();
        async move {
            (
                axum::http::StatusCode::TEMPORARY_REDIRECT,
                [(axum::http::header::LOCATION, location)],
            )
        }
    });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let (endpoint, _secret) = create_endpoint(&env, &redirector_url, vec![]).await;
    insert_deliveries(&env, endpoint.id, 1).await;

    worker(&env).run_tick().await;

    assert!(
        target.requests().is_empty(),
        "the redirect target was never called"
    );
    let delivery = &deliveries(&env, endpoint.id).await[0].delivery;
    assert_eq!(delivery.last_response_status, Some(307));
    assert_ne!(delivery.status, WebhookDeliveryStatus::Succeeded);
}

/// With private destinations allowed (as in tests), an endpoint could point
/// at an internal service: its answers must not be readable back from the delivery log.
#[rstest]
#[tokio::test]
async fn no_response_body_is_kept_when_private_destinations_are_allowed(
    #[future] test_env_minimal: TestEnv,
) {
    let env = test_env_minimal.await;
    assert!(
        core_egress(&env)
            .client(EgressPurpose::Webhooks)
            .expect("egress client")
            .allows_private_destinations()
    );
    let receiver = Receiver::spawn(vec![]).await;
    let (endpoint, _secret) = create_endpoint(&env, &receiver.url, vec![]).await;
    insert_deliveries(&env, endpoint.id, 1).await;

    worker(&env).run_tick().await;

    let delivery = &deliveries(&env, endpoint.id).await[0].delivery;
    let details = env
        .store()
        .get_webhook_delivery(TENANT_ID, delivery.id)
        .await
        .expect("delivery details");
    assert_eq!(details.attempts[0].response_status, Some(200));
    assert_eq!(
        details.attempts[0].response_body.as_deref().unwrap_or(""),
        ""
    );
}

/// An endpoint migrated from Svix also gets the same id, timestamp and signature under the
/// `svix-*` names its receiver may still verify.
#[rstest]
#[tokio::test]
async fn a_migrated_endpoint_also_gets_the_svix_headers(#[future] test_env_minimal: TestEnv) {
    let env = test_env_minimal.await;
    let receiver = Receiver::spawn(vec![]).await;
    let (endpoint, secret) = create_endpoint(&env, &receiver.url, vec![]).await;

    diesel::sql_query("UPDATE webhook_out_endpoint SET legacy_svix_headers = true WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(*endpoint.id)
        .execute(&mut env.conn().await)
        .await
        .expect("mark the endpoint as migrated");

    create_customer(&env, "Legacy Headers Co").await;
    fan_out(&env).await;
    assert_eq!(worker(&env).run_tick().await.succeeded, 1);

    let requests = receiver.requests();
    assert_eq!(requests.len(), 1, "exactly one POST");
    let request = &requests[0];

    for (standard, legacy) in [
        ("webhook-id", "svix-id"),
        ("webhook-timestamp", "svix-timestamp"),
        ("webhook-signature", "svix-signature"),
    ] {
        assert_eq!(request.header(legacy), request.header(standard), "{legacy}");
    }
    assert_eq!(
        request.header("svix-signature"),
        expected_signature(
            &secret,
            request.header("webhook-id"),
            request.header("webhook-timestamp"),
            &request.body,
        )
    );
}
