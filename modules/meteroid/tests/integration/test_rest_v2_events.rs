//! `POST /api/v2/events/ingest` (ADR-0013 §13): v2's body (`customer` an id or `ext:<alias>`,
//! properties JSON scalars) and result (`accepted`, `rejected` at their pointers), on v1's
//! metering path; v1 unchanged.
use crate::data::ids::CUST_UBER_ID;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;
use crate::test_rest_v2::Fixture;
use common_domain::ids::{CustomerId, TenantId};
use metering_grpc::meteroid::metering::v1::event::CustomerId as EventCustomer;
use meteroid_grpc::meteroid::api;
use meteroid_store::StoreResult;
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, IngestEventsFailure, IngestEventsRequest,
    IngestEventsResult, MockUsageClient, UsageClient, UsageData, WindowedUsageData,
};
use meteroid_store::domain::{BillableMetric, UsagePeriod};
use meteroid_store::errors::StoreError;
use meteroid_testkit::mailer::MockMailerService;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Metering that takes every event but those whose id starts with `bad`, reported one by one; as
/// metering does, a batch with one is taken whole or not at all (`fail_on_error`), its failures
/// answered either way. An event whose id starts with `late` finds metering not answering in time.
struct Ingesting {
    inner: MockUsageClient,
    ingested: Mutex<Vec<String>>,
    /// Each ingested event's customer as metering is handed it: `id:<id>` or `alias:<alias>`.
    customers: Mutex<Vec<String>>,
    /// Each ingested event's properties as metering is handed them.
    properties: Mutex<Vec<HashMap<String, String>>>,
}

#[async_trait::async_trait]
impl UsageClient for Ingesting {
    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.inner
            .fetch_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.inner
            .fetch_usage_grouped_by(tenant_id, customer_id, metric, group_by, period)
            .await
    }

    async fn fetch_total_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        self.inner
            .fetch_total_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        self.inner
            .fetch_windowed_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_usage_summary(
        &self,
        tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.inner
            .fetch_usage_summary(tenant_id, customer_id, metric, period)
            .await
    }

    async fn search_events(
        &self,
        tenant_id: &TenantId,
        options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        self.inner.search_events(tenant_id, options).await
    }

    async fn ingest_events(
        &self,
        _tenant_id: &TenantId,
        request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        if request
            .events
            .iter()
            .any(|event| event.id.starts_with("late"))
        {
            // What the metering client reports when its deadline passes.
            return Err(error_stack::Report::new(tonic::Status::deadline_exceeded(
                "ingest_events timed out",
            ))
            .change_context(StoreError::MeteringServiceError));
        }
        let (refused, taken): (Vec<_>, Vec<_>) = request
            .events
            .into_iter()
            .partition(|event| event.id.starts_with("bad"));
        let failures: Vec<IngestEventsFailure> = refused
            .into_iter()
            .map(|event| IngestEventsFailure {
                event_id: event.id,
                reason: "refused".to_owned(),
            })
            .collect();
        if request.fail_on_error && !failures.is_empty() {
            return Ok(IngestEventsResult { failures });
        }
        self.customers
            .lock()
            .unwrap()
            .extend(taken.iter().map(|event| match &event.customer_id {
                Some(EventCustomer::MeteroidCustomerId(id)) => format!("id:{id}"),
                Some(EventCustomer::ExternalCustomerAlias(alias)) => format!("alias:{alias}"),
                None => "none".to_owned(),
            }));
        self.properties
            .lock()
            .unwrap()
            .extend(taken.iter().map(|event| event.properties.clone()));
        self.ingested
            .lock()
            .unwrap()
            .extend(taken.into_iter().map(|event| event.id));
        Ok(IngestEventsResult { failures })
    }
}

async fn fixture(usage: Arc<Ingesting>) -> Fixture {
    crate::helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        database,
        SeedLevel::PLANS,
        usage,
        Arc::new(MockMailerService::new()),
    )
    .await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let api_key = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    )
    .api_tokens
    .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
        name: "api-v2-events".to_owned(),
        ..Default::default()
    })
    .await
    .unwrap()
    .into_inner()
    .api_key;
    Fixture {
        setup,
        client: http_client_tracing::client(),
        api_key,
    }
}

fn event(id: &str) -> serde_json::Value {
    json!({
        "event_id": id,
        "code": "api_calls",
        "customer": "ext:uber",
        "properties": {"tokens": 12, "model": "large", "cached": false},
    })
}

fn v1_event(id: &str) -> serde_json::Value {
    json!({
        "event_id": id,
        "code": "api_calls",
        "customer_id": "uber",
        "properties": {"tokens": "12"},
    })
}

/// Rules: ADR-0013 §13
#[tokio::test]
async fn test_rest_v2_ingests_events_in_v2s_shape() {
    let usage = Arc::new(Ingesting {
        inner: MockUsageClient::noop(),
        ingested: Mutex::new(Vec::new()),
        customers: Mutex::new(Vec::new()),
        properties: Mutex::new(Vec::new()),
    });
    let f = fixture(usage.clone()).await;
    let path = "/api/v2/events/ingest";

    // Accepted: every event counted, none rejected; scalar properties handed to metering as text.
    let accepted = f
        .post(path, json!({"events": [event("evt-1"), event("evt-2")]}))
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{:#}", accepted.body);
    assert_eq!(accepted.body, json!({"accepted": 2, "rejected": []}));
    assert_eq!(accepted.header("Meteroid-Version"), Some("2026-12-01"));
    assert_eq!(*usage.ingested.lock().unwrap(), vec!["evt-1", "evt-2"]);
    let properties = usage.properties.lock().unwrap()[0].clone();
    assert_eq!(properties["tokens"], "12");
    assert_eq!(properties["model"], "large");
    assert_eq!(properties["cached"], "false");

    // Partial failures: metering's and v2's own rejections, each at its pointer, in batch order.
    let partial = f
        .post(
            path,
            json!({"events": [
                event("evt-3"),
                event("bad-1"),
                {"event_id": "evt-null", "code": "api_calls", "customer": "ext:uber", "properties": {"tokens": null}},
                {"event_id": "evt-bare", "code": "api_calls", "customer": "uber"}
            ], "allow_partial_failures": true}),
        )
        .await;
    assert_eq!(partial.status, StatusCode::OK, "{:#}", partial.body);
    assert_eq!(partial.body["accepted"], 1, "{:#}", partial.body);
    let rejected = partial.body["rejected"].as_array().expect("rejected");
    let at: Vec<(&str, &str, u64)> = rejected
        .iter()
        .map(|rejection| {
            (
                rejection["event_id"].as_str().unwrap(),
                rejection["path"].as_str().unwrap(),
                rejection["index"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        at,
        vec![
            ("bad-1", "/events/1", 1),
            ("evt-null", "/events/2/properties/tokens", 2),
            ("evt-bare", "/events/3/customer", 3),
        ]
    );
    assert_eq!(rejected[0]["message"], "refused");
    assert!(
        usage
            .ingested
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == "evt-3")
    );

    // A batch refused whole (metering answers its failures, ingesting none): a problem, each
    // rejection at its event's pointer.
    let refused = f
        .post(path, json!({"events": [event("evt-4"), event("bad-2")]}))
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    assert_eq!(
        refused.header("content-type"),
        Some("application/problem+json")
    );
    assert_eq!(refused.body["code"], "validation_failed");
    assert_eq!(refused.body["errors"][0]["path"], "/events/1");
    assert_eq!(refused.body["errors"][0]["code"], "invalid");
    assert_eq!(refused.body["errors"][0]["message"], "bad-2: refused");
    assert_eq!(
        refused.body["errors"].as_array().map(Vec::len),
        Some(1),
        "{:#}",
        refused.body
    );
    assert!(refused.body["request_id"].is_string(), "{:#}", refused.body);
    assert!(
        !usage
            .ingested
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == "evt-4"),
        "nothing of a refused batch is ingested"
    );

    // An event v2 rejects itself refuses the batch before metering is asked.
    let object = f
        .post(
            path,
            json!({"events": [event("evt-11"), {"event_id": "evt-12", "code": "api_calls", "customer": "ext:uber", "properties": {"tags": ["a"]}}]}),
        )
        .await;
    assert_eq!(
        object.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        object.body
    );
    assert_eq!(
        object.body["errors"][0]["path"],
        "/events/1/properties/tags"
    );
    assert!(
        !usage
            .ingested
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == "evt-11"),
        "metering is not asked"
    );

    // Metering not answering in time is unavailable, retried: never an internal error.
    let late = f.post(path, json!({"events": [event("late-1")]})).await;
    assert_eq!(
        late.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{:#}",
        late.body
    );
    assert_eq!(late.body["code"], "unavailable");

    // The batch's size and each event's fields, refused at their pointers.
    let empty = f.post(path, json!({"events": []})).await;
    assert_eq!(
        empty.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        empty.body
    );
    assert_eq!(empty.body["code"], "validation_failed");
    assert_eq!(empty.body["errors"][0]["path"], "/events");
    let many: Vec<serde_json::Value> = (0..501).map(|i| event(&format!("evt-many-{i}"))).collect();
    let too_many = f.post(path, json!({"events": many})).await;
    assert_eq!(too_many.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(too_many.body["errors"][0]["path"], "/events");
    let most: Vec<serde_json::Value> = (0..500).map(|i| event(&format!("evt-most-{i}"))).collect();
    let at_most = f.post(path, json!({"events": most})).await;
    assert_eq!(at_most.status, StatusCode::OK, "{:#}", at_most.body);
    assert_eq!(at_most.body["accepted"], 500);
    let unnamed = f.post(path, json!({"events": [event("")]})).await;
    assert_eq!(
        unnamed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        unnamed.body
    );
    assert_eq!(unnamed.body["errors"][0]["path"], "/events/0/event_id");

    // `ext:<alias>` is the alias whatever it looks like; an id is an id.
    usage.customers.lock().unwrap().clear();
    let uber = CUST_UBER_ID.to_string();
    let named = f
        .post(
            path,
            json!({"events": [
                {"event_id": "evt-ext-1", "code": "api_calls", "customer": "ext:uber"},
                {"event_id": "evt-ext-2", "code": "api_calls", "customer": format!("ext:{uber}")},
                {"event_id": "evt-ext-3", "code": "api_calls", "customer": &uber}
            ]}),
        )
        .await;
    assert_eq!(named.status, StatusCode::OK, "{:#}", named.body);
    assert_eq!(
        *usage.customers.lock().unwrap(),
        vec![
            "alias:uber".to_owned(),
            format!("alias:{uber}"),
            format!("id:{uber}"),
        ]
    );
    let aliasless = f
        .post(
            path,
            json!({"events": [event("evt-7"), {"event_id": "evt-8", "code": "api_calls", "customer": "ext:"}]}),
        )
        .await;
    assert_eq!(
        aliasless.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        aliasless.body
    );
    assert_eq!(aliasless.body["errors"][0]["path"], "/events/1/customer");

    // v1's body is not v2's: an unknown field is unreadable.
    let v1_body = f.post(path, json!({"events": [v1_event("evt-13")]})).await;
    assert_eq!(
        v1_body.status,
        StatusCode::BAD_REQUEST,
        "{:#}",
        v1_body.body
    );
    assert_eq!(v1_body.body["code"], "invalid_request");

    let unreadable = f
        .send(
            f.client
                .post(f.url(path))
                .bearer_auth(&f.api_key)
                .header("content-type", "application/json")
                .body("{"),
        )
        .await;
    assert_eq!(unreadable.status, StatusCode::BAD_REQUEST);
    assert_eq!(unreadable.body["code"], "invalid_request");

    let anonymous = f
        .send(
            f.client
                .post(f.url(path))
                .json(&json!({"events": [event("evt-5")]})),
        )
        .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.body["code"], "unauthorized");

    // v1 is unchanged.
    let v1 = f
        .send(
            f.client
                .post(f.url("/api/v1/events/ingest"))
                .bearer_auth(&f.api_key)
                .json(&json!({"events": [v1_event("evt-6")]})),
        )
        .await;
    assert_eq!(v1.status, StatusCode::OK, "{:#}", v1.body);
    assert_eq!(v1.body, json!({}));
    // v1 still answers a batch refused whole with its failures, in a 200.
    let v1_refused = f
        .send(
            f.client
                .post(f.url("/api/v1/events/ingest"))
                .bearer_auth(&f.api_key)
                .json(&json!({"events": [v1_event("evt-10"), v1_event("bad-3")]})),
        )
        .await;
    assert_eq!(v1_refused.status, StatusCode::OK, "{:#}", v1_refused.body);
    assert_eq!(
        v1_refused.body,
        json!({"failures": [{"event_id": "bad-3", "reason": "refused"}]})
    );
    usage.customers.lock().unwrap().clear();
    let v1_prefixed = f
        .send(
            f.client
                .post(f.url("/api/v1/events/ingest"))
                .bearer_auth(&f.api_key)
                .json(&json!({"events": [
                    {"event_id": "evt-9", "code": "api_calls", "customer_id": "ext:uber"},
                    {"event_id": "evt-14", "code": "api_calls", "customer_id": "uber"}
                ]})),
        )
        .await;
    assert_eq!(v1_prefixed.status, StatusCode::OK, "{:#}", v1_prefixed.body);
    assert_eq!(
        *usage.customers.lock().unwrap(),
        vec!["alias:ext:uber".to_owned(), "alias:uber".to_owned()],
        "v1 reads the prefix as part of the alias, and a bare value as an alias"
    );

    let spec = f
        .send(f.client.get(f.url("/api-docs/v2/openapi.json")))
        .await;
    assert!(spec.body["paths"][path]["post"].is_object());
}

fn segment_track(message_id: &str, group: Option<&str>) -> serde_json::Value {
    let mut event = json!({
        "messageId": message_id,
        "type": "track",
        "event": "api_calls",
        "userId": "user-7",
        "properties": {"tokens": 12, "model": {"name": "large"}},
        "context": {"library": {"name": "analytics.js"}},
        "anonymousId": "anon-1",
    });
    if let Some(group) = group {
        event["context"]["groupId"] = json!(group);
    }
    event
}

#[tokio::test]
async fn test_rest_v2_ingests_segment_track_events() {
    let usage = Arc::new(Ingesting {
        inner: MockUsageClient::noop(),
        ingested: Mutex::new(Vec::new()),
        customers: Mutex::new(Vec::new()),
        properties: Mutex::new(Vec::new()),
    });
    let f = fixture(usage.clone()).await;
    let path = "/api/v2/events/segment";

    // A batch: the group names the customer; refusals at their pointer, the rest ingested.
    let batch = f
        .post(
            path,
            json!([
                segment_track("seg-1", Some("uber")),
                segment_track("seg-2", None),
                segment_track("bad-seg", Some("uber")),
                {"messageId": "seg-identify", "type": "identify", "userId": "user-7", "event": "x"},
            ]),
        )
        .await;
    assert_eq!(batch.status, StatusCode::OK, "{:#}", batch.body);
    assert_eq!(batch.body["accepted"], 1, "{:#}", batch.body);
    let at: Vec<&str> = batch.body["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rejection| rejection["path"].as_str().unwrap())
        .collect();
    assert_eq!(at, vec!["/1/context/groupId", "/2", "/3/type"]);
    assert_eq!(*usage.ingested.lock().unwrap(), vec!["seg-1"]);
    assert_eq!(*usage.customers.lock().unwrap(), vec!["alias:uber"]);
    let properties = usage.properties.lock().unwrap()[0].clone();
    assert_eq!(properties["tokens"], "12");
    assert_eq!(properties["model.name"], "large");

    // One event, by its user.
    let one = f
        .post(
            &format!("{path}?customer=user"),
            segment_track("seg-3", None),
        )
        .await;
    assert_eq!(one.status, StatusCode::OK, "{:#}", one.body);
    assert_eq!(one.body, json!({"accepted": 1, "rejected": []}));
    assert_eq!(usage.customers.lock().unwrap()[1], "alias:user-7");

    // More than metering's 500 a call: sent in chunks, every event counted.
    let large: Vec<serde_json::Value> = (0..1200)
        .map(|i| {
            json!({"messageId": format!("seg-big-{i}"), "event": "api_calls", "context": {"groupId": "uber"}})
        })
        .collect();
    let chunked = f.post(path, json!(large)).await;
    assert_eq!(chunked.status, StatusCode::OK, "{:#}", chunked.body);
    assert_eq!(chunked.body["accepted"], 1200);
    assert_eq!(usage.ingested.lock().unwrap().len(), 1202);

    let empty = f.post(path, json!([])).await;
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);
    let scalar = f.post(path, json!("track")).await;
    assert_eq!(scalar.status, StatusCode::UNPROCESSABLE_ENTITY);
}
