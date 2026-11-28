//! AI gateways' OTLP traces and LiteLLM logs through the REST API, as the LLM usage convention's
//! events: idempotent, rejections reported. The end-to-end test runs real LiteLLM proxies
//! (`gateway_e2e/docker-compose.yml`).
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
    IngestEventsResult, UsageClient, UsageData, WindowedUsageData,
};
use meteroid_store::domain::{BillableMetric, UsagePeriod};
use meteroid_store::errors::StoreError;
use meteroid_testkit::mailer::MockMailerService;
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value};
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
use prost::Message;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const OTLP: &str = "/api/v2/events/otlp/v1/traces";
const LITELLM: &str = "/api/v2/events/litellm";

/// An event as metering keeps it.
#[derive(Clone, Debug, PartialEq)]
struct Stored {
    id: String,
    code: String,
    customer: String,
    properties: BTreeMap<String, String>,
}

/// Metering keeping the first copy of an event by `(id, customer)`, as the real one does; it does
/// not know the customer `unknown`, and answers `unavailable` to its first `failing` batches.
#[derive(Default)]
struct Metering {
    stored: Mutex<Vec<Stored>>,
    batches: AtomicUsize,
    failing: usize,
}

impl Metering {
    fn stored(&self) -> Vec<Stored> {
        self.stored.lock().unwrap().clone()
    }

    fn of_code(&self, code: &str) -> Vec<Stored> {
        self.stored()
            .into_iter()
            .filter(|event| event.code == code)
            .collect()
    }
}

fn unread<T>() -> StoreResult<T> {
    Err(error_stack::Report::new(StoreError::InvalidArgument(
        "not read here".to_owned(),
    )))
}

#[async_trait::async_trait]
impl UsageClient for Metering {
    async fn fetch_usage(
        &self,
        _: &TenantId,
        _: &CustomerId,
        _: &BillableMetric,
        _: UsagePeriod,
    ) -> StoreResult<UsageData> {
        unread()
    }

    async fn fetch_usage_grouped_by(
        &self,
        _: &TenantId,
        _: &CustomerId,
        _: &BillableMetric,
        _: &str,
        _: UsagePeriod,
    ) -> StoreResult<UsageData> {
        unread()
    }

    async fn fetch_total_usage(
        &self,
        _: &TenantId,
        _: &CustomerId,
        _: &BillableMetric,
        _: UsagePeriod,
    ) -> StoreResult<Decimal> {
        unread()
    }

    async fn fetch_windowed_usage(
        &self,
        _: &TenantId,
        _: &CustomerId,
        _: &BillableMetric,
        _: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        unread()
    }

    async fn fetch_usage_summary(
        &self,
        _: &TenantId,
        _: Option<&CustomerId>,
        _: &BillableMetric,
        _: UsagePeriod,
    ) -> StoreResult<UsageData> {
        unread()
    }

    async fn search_events(
        &self,
        _: &TenantId,
        _: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        unread()
    }

    async fn ingest_events(
        &self,
        _: &TenantId,
        request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        if self.batches.fetch_add(1, Ordering::SeqCst) < self.failing {
            return Err(error_stack::Report::new(StoreError::UsageUnavailable));
        }
        let mut failures = Vec::new();
        let mut stored = self.stored.lock().unwrap();
        for event in request.events {
            let customer = match event.customer_id {
                Some(EventCustomer::ExternalCustomerAlias(alias)) => alias,
                Some(EventCustomer::MeteroidCustomerId(id)) => id,
                None => String::new(),
            };
            if customer == "unknown" {
                failures.push(IngestEventsFailure {
                    event_id: event.id,
                    reason: "Customer not found".to_owned(),
                });
                continue;
            }
            if stored
                .iter()
                .any(|kept| kept.id == event.id && kept.customer == customer)
            {
                continue;
            }
            stored.push(Stored {
                id: event.id,
                code: event.code,
                customer,
                properties: event.properties.into_iter().collect(),
            });
        }
        Ok(IngestEventsResult { failures })
    }
}

async fn fixture(metering: Arc<Metering>) -> Fixture {
    crate::helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid_with_clients(
        database,
        SeedLevel::PLANS,
        metering,
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
    .clone()
    .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
        name: "ai-gateway".to_owned(),
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

fn attribute(key: &str, value: any_value::Value) -> KeyValue {
    KeyValue {
        key: key.to_owned(),
        value: Some(AnyValue { value: Some(value) }),
        ..Default::default()
    }
}

fn usage_span(span_id: u8, customer: Option<&str>, padding: usize) -> Span {
    let mut attributes = vec![
        attribute(
            "gen_ai.request.model",
            any_value::Value::StringValue("gpt-4.1".to_owned()),
        ),
        attribute("gen_ai.usage.input_tokens", any_value::Value::IntValue(50)),
        attribute("gen_ai.usage.output_tokens", any_value::Value::IntValue(5)),
        attribute(
            "http.user_agent",
            any_value::Value::StringValue("x".repeat(padding)),
        ),
    ];
    if let Some(customer) = customer {
        attributes.push(attribute(
            "meteroid.customer",
            any_value::Value::StringValue(customer.to_owned()),
        ));
    }
    Span {
        trace_id: vec![7; 16],
        span_id: vec![span_id; 8],
        end_time_unix_nano: u64::try_from(chrono::Utc::now().timestamp_nanos_opt().unwrap())
            .unwrap(),
        attributes,
        ..Default::default()
    }
}

fn traces(spans: Vec<Span>) -> ExportTraceServiceRequest {
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans,
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

impl Fixture {
    async fn otlp(
        &self,
        body: Vec<u8>,
        content_type: &str,
        headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut request = self
            .client
            .post(self.url(OTLP))
            .bearer_auth(&self.api_key)
            .header("content-type", content_type)
            .body(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await.unwrap()
    }
}

#[tokio::test]
async fn test_rest_v2_gateway_otlp_traces() {
    let metering = Arc::new(Metering::default());
    let f = fixture(metering.clone()).await;

    // LiteLLM 1.111.0's OTel v2 export of one OpenAI call: 1000 in (200 cached), 300 out.
    let litellm = include_bytes!(
        "../../crates/meteroid-rest/src/v2/gateway/fixtures/litellm-1.111.0-otel-v2.binpb"
    )
    .to_vec();
    for _ in 0..2 {
        let answer = f.otlp(litellm.clone(), "application/x-protobuf", &[]).await;
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(answer.headers()["content-type"], "application/x-protobuf");
        let response = ExportTraceServiceResponse::decode(answer.bytes().await.unwrap()).unwrap();
        assert_eq!(response.partial_success, None, "every span is accepted");
    }
    let stored = metering.of_code("llm_tokens");
    let request_id = &stored[0].properties["request_id"];
    assert_eq!(
        stored
            .iter()
            .map(|event| (event.id.clone(), event.properties["tokens"].clone()))
            .collect::<Vec<_>>(),
        [
            (format!("{request_id}:input"), "800".to_owned()),
            (format!("{request_id}:output"), "300".to_owned()),
            (format!("{request_id}:cache_read"), "200".to_owned()),
        ],
        "sent twice, billed once"
    );
    assert!(stored.iter().all(|event| event.customer == "acme-gmbh"));
    assert_eq!(stored[0].properties["model"], "gpt-mock");
    assert_eq!(stored[0].properties["cost_reported"], "0");

    // OTLP/JSON; one span names an unknown customer, one none; another code and attribute.
    let request = traces(vec![
        usage_span(1, Some("acme"), 0),
        usage_span(2, Some("unknown"), 0),
        usage_span(3, None, 0),
    ]);
    let answer = f
        .otlp(
            serde_json::to_vec(&request).unwrap(),
            "application/json",
            &[("meteroid-event-code", "gateway_tokens")],
        )
        .await;
    assert_eq!(answer.status(), StatusCode::OK);
    assert_eq!(answer.headers()["content-type"], "application/json");
    let body: Value = answer.json().await.unwrap();
    assert_eq!(body["partialSuccess"]["rejectedSpans"], 2, "{body}");
    let message = body["partialSuccess"]["errorMessage"].as_str().unwrap();
    assert!(
        message.contains("-0202020202020202:input: Customer not found"),
        "{message}"
    );
    assert!(
        message.contains("Span 0303030303030303: names no customer"),
        "{message}"
    );
    let stored = metering.of_code("gateway_tokens");
    assert_eq!(
        stored.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        [
            "07070707070707070707070707070707-0101010101010101:input",
            "07070707070707070707070707070707-0101010101010101:output"
        ]
    );

    // An export larger than an API call's 1 MB.
    let large = traces(vec![usage_span(4, Some("acme"), 2 * 1024 * 1024)]);
    let answer = f
        .otlp(large.encode_to_vec(), "application/x-protobuf", &[])
        .await;
    assert_eq!(answer.status(), StatusCode::OK);

    let refused = [
        (
            f.client
                .post(f.url(OTLP))
                .header("content-type", "application/x-protobuf")
                .body(litellm.clone()),
            StatusCode::UNAUTHORIZED,
        ),
        (
            f.client
                .post(f.url(OTLP))
                .bearer_auth(&f.api_key)
                .header("content-type", "text/plain")
                .body("hello"),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            f.client
                .post(f.url(OTLP))
                .bearer_auth(&f.api_key)
                .header("content-type", "application/x-protobuf")
                .header("meteroid-event-code", "c".repeat(600))
                .body(litellm.clone()),
            StatusCode::BAD_REQUEST,
        ),
    ];
    for (request, status) in refused {
        let answer = request.send().await.unwrap();
        assert_eq!(answer.status(), status, "{}", answer.text().await.unwrap());
    }
}

#[tokio::test]
async fn test_rest_v2_gateway_otlp_retried_when_metering_is_down() {
    let metering = Arc::new(Metering {
        failing: 1,
        ..Default::default()
    });
    let f = fixture(metering.clone()).await;
    let body = traces(vec![usage_span(1, Some("acme"), 0)]).encode_to_vec();
    let first = f.otlp(body.clone(), "application/x-protobuf", &[]).await;
    assert_eq!(
        first.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an exporter retries a 503"
    );
    let second = f.otlp(body, "application/x-protobuf", &[]).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(metering.stored().len(), 2);
}

#[tokio::test]
async fn test_rest_v2_gateway_litellm_logs() {
    let metering = Arc::new(Metering::default());
    let f = fixture(metering.clone()).await;
    let generic: Value = serde_json::from_str(include_str!(
        "../../crates/meteroid-rest/src/v2/gateway/fixtures/litellm-1.111.0-generic-api.json"
    ))
    .unwrap();
    let mut batch = generic.as_array().unwrap().clone();
    let mut unknown = batch[0].clone();
    unknown["litellm_call_id"] = json!("call-unknown");
    unknown["end_user"] = json!("unknown");
    let mut failed = batch[0].clone();
    failed["status"] = json!("failure");
    batch.extend([unknown, failed, json!({"litellm_call_id": "anonymous"})]);

    for _ in 0..2 {
        let answer = f
            .client
            .post(f.url(LITELLM))
            .bearer_auth(&f.api_key)
            .json(&batch)
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        let body: Value = answer.json().await.unwrap();
        assert_eq!(body["calls"], 1, "{body}");
        assert_eq!(body["ignored"], 1);
        assert_eq!(body["rejected"][0]["index"], 1);
        assert_eq!(
            body["rejected"][0]["message"],
            "call-unknown:input: Customer not found"
        );
        assert_eq!(body["rejected"][1]["index"], 3);
    }
    let golden: Value = serde_json::from_str(include_str!(
        "../../crates/meteroid-rest/src/v2/gateway/fixtures/litellm-callback-events.json"
    ))
    .unwrap();
    let expected: Vec<Stored> = golden["events"]["generic_api"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| Stored {
            id: event["event_id"].as_str().unwrap().to_owned(),
            code: event["code"].as_str().unwrap().to_owned(),
            customer: event["customer"]
                .as_str()
                .unwrap()
                .trim_start_matches("ext:")
                .to_owned(),
            properties: serde_json::from_value(event["properties"].clone()).unwrap(),
        })
        .collect();
    assert_eq!(
        metering.stored(),
        expected,
        "the removed callback's events, once"
    );
}

/// What the end-to-end proxy's provider answers: OpenAI usage with cached and reasoning tokens.
async fn mock_provider() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().fallback(axum::routing::post(|| async {
        axum::Json(json!({
            "id": "chatcmpl-mock",
            "object": "chat.completion",
            "created": 1795000000,
            "model": "gpt-mock",
            "choices": [{"index": 0, "finish_reason": "stop",
                "message": {"role": "assistant", "content": "SECRET ANSWER"}}],
            "usage": {"prompt_tokens": 1000, "completion_tokens": 300, "total_tokens": 1300,
                "prompt_tokens_details": {"cached_tokens": 200},
                "completion_tokens_details": {"reasoning_tokens": 100}}
        }))
    }));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

fn compose(project: &str, env: &HashMap<&str, String>, args: &[&str]) {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/integration/gateway_e2e/docker-compose.yml"
    );
    let status = std::process::Command::new("docker")
        .args(["compose", "-f", file, "-p", project])
        .args(args)
        .envs(env)
        .status()
        .expect("docker compose");
    assert!(status.success(), "docker compose {args:?}");
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Real LiteLLM proxies (the pinned image), one exporting OTLP traces, the other sending its
/// generic logger's batches; metering refuses the first batch, so one is sent again. Needs
/// Docker: `cargo test -p meteroid --test integration gateway_e2e -- --ignored`.
#[tokio::test]
#[ignore]
async fn test_rest_v2_gateway_e2e_litellm() {
    let metering = Arc::new(Metering {
        failing: 1,
        ..Default::default()
    });
    let f = fixture(metering.clone()).await;
    let provider = mock_provider().await;
    let (otel_port, logger_port) = (free_port(), free_port());
    let project = format!("meteroid-gateway-e2e-{otel_port}");
    let env = HashMap::from([
        ("METEROID_API_BASE", f.url("")),
        ("METEROID_API_KEY", f.api_key.clone()),
        ("PROVIDER_PORT", provider.to_string()),
        ("OTEL_PROXY_PORT", otel_port.to_string()),
        ("LOGGER_PROXY_PORT", logger_port.to_string()),
    ]);
    compose(&project, &env, &["up", "-d"]);
    let result = async {
        for (port, customer) in [(otel_port, "acme-otel"), (logger_port, "acme-logger")] {
            let proxy = format!("http://127.0.0.1:{port}");
            let mut ready = false;
            for _ in 0..90 {
                if let Ok(answer) = f
                    .client
                    .get(format!("{proxy}/health/liveliness"))
                    .send()
                    .await
                    && answer.status().is_success()
                {
                    ready = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            assert!(ready, "the LiteLLM proxy on {port} did not start");
            let answer = f
                .client
                .post(format!("{proxy}/v1/chat/completions"))
                .bearer_auth("sk-e2e")
                .json(&json!({"model": "gpt-mock", "user": customer,
                    "messages": [{"role": "user", "content": "Hello"}]}))
                .send()
                .await
                .unwrap();
            assert_eq!(answer.status(), StatusCode::OK);
        }
        let of = |customer: &str| -> Vec<Stored> {
            metering
                .stored()
                .into_iter()
                .filter(|event| event.customer == customer)
                .collect()
        };
        for _ in 0..60 {
            if of("acme-otel").len() == 3 && of("acme-logger").len() == 4 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let tokens = |events: &[Stored]| {
            events
                .iter()
                .map(|e| format!("{} {}", e.properties["token_class"], e.properties["tokens"]))
                .collect::<Vec<_>>()
        };
        let (traced, logged) = (of("acme-otel"), of("acme-logger"));
        assert_eq!(
            tokens(&logged),
            ["input 800", "output 200", "cache_read 200", "reasoning 100"],
            "the generic logger: {logged:?}"
        );
        assert_eq!(
            tokens(&traced),
            ["input 800", "output 300", "cache_read 200"],
            "OTLP, LiteLLM sends no reasoning count: {traced:?}"
        );
        for events in [&traced, &logged] {
            let call_id = &events[0].properties["request_id"];
            assert_eq!(call_id.len(), 36, "LiteLLM's call id: {call_id}");
            for event in events.iter() {
                assert_eq!(event.code, "llm_tokens");
                assert_eq!(event.properties["model"], "gpt-mock");
                assert_eq!(event.properties["provider"], "openai");
                assert_eq!(
                    event.id,
                    format!("{call_id}:{}", event.properties["token_class"])
                );
                assert!(
                    !event
                        .properties
                        .values()
                        .any(|value| value.contains("SECRET") || value.contains("Hello")),
                    "no content is billed"
                );
            }
        }
        assert!(
            metering.batches.load(Ordering::SeqCst) >= 3,
            "the batch refused was sent again"
        );
    };
    let outcome = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(result)).await;
    compose(&project, &env, &["down", "--timeout", "5"]);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
