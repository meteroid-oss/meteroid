//! OTLP/HTTP traces (protobuf or JSON, gzip or not) whose spans carry the OpenTelemetry GenAI
//! usage attributes: each such span is one LLM call; the gateway's other spans are ignored.
use super::{LlmCall, Record, Settings, Tokens, decimal_text};
use crate::AppState;
use crate::v2::problem::{Problem, ProblemCode};
use axum::Extension;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use chrono::DateTime;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::{KeyValue, any_value};
use opentelemetry_proto::tonic::trace::v1::Span;
use prost::Message;
use std::collections::HashMap;
use std::io::Read;
use utoipa::ToSchema;

/// The largest body once decompressed.
pub const MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;
/// The rejections an answer spells out; the others are counted.
const MAX_REASONS: usize = 20;

/// The customer attributes read by default, each on the span then its resource: Meteroid's own,
/// then LiteLLM's organization alias, end user and team alias (its callback's default).
pub const DEFAULT_CUSTOMER_FROM: [&str; 7] = [
    "meteroid.customer",
    "metadata.user_api_key_customer_org_alias",
    "litellm.end_user.id",
    "litellm.metadata.user_api_key_end_user_id",
    "metadata.user_api_key_end_user_id",
    "litellm.team.alias",
    "metadata.user_api_key_team_alias",
];
/// Request ids looked for by default; `<trace id>-<span id>` without one.
pub const DEFAULT_REQUEST_ID_FROM: [&str; 2] = ["litellm.call_id", "gen_ai.request.id"];

const INPUT_TOKENS: [&str; 2] = ["gen_ai.usage.input_tokens", "gen_ai.usage.prompt_tokens"];
const OUTPUT_TOKENS: [&str; 2] = [
    "gen_ai.usage.output_tokens",
    "gen_ai.usage.completion_tokens",
];
const CACHE_READ_TOKENS: &str = "gen_ai.usage.cache_read.input_tokens";
const CACHE_WRITE_TOKENS: &str = "gen_ai.usage.cache_creation.input_tokens";
const REASONING_TOKENS: &str = "gen_ai.usage.reasoning.output_tokens";
const MODEL: [&str; 3] = [
    "litellm.model_group",
    "gen_ai.request.model",
    "gen_ai.response.model",
];
const PROVIDER: [&str; 2] = ["gen_ai.provider.name", "gen_ai.system"];
const COST: [&str; 4] = [
    "litellm.cost.total",
    "gen_ai.cost.total_cost",
    "gen_ai.usage.cost",
    "tensorzero.cost_usd",
];
const PAYER: &str = "meteroid.payer";

type Attributes<'a> = HashMap<&'a str, &'a any_value::Value>;

fn attributes(list: &[KeyValue]) -> Attributes<'_> {
    list.iter()
        .filter_map(|kv| Some((kv.key.as_str(), kv.value.as_ref()?.value.as_ref()?)))
        .collect()
}

fn text_of(value: &any_value::Value) -> Option<String> {
    let text = match value {
        any_value::Value::StringValue(text) => text.trim().to_owned(),
        any_value::Value::IntValue(number) => number.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn count_of(value: &any_value::Value) -> Option<u64> {
    match value {
        any_value::Value::IntValue(number) => u64::try_from(*number).ok(),
        any_value::Value::DoubleValue(number) if *number >= 0.0 => Some(*number as u64),
        any_value::Value::StringValue(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn number_text(value: &any_value::Value) -> Option<String> {
    match value {
        any_value::Value::DoubleValue(number) if number.is_finite() => {
            decimal_text(&number.to_string())
        }
        any_value::Value::IntValue(number) => Some(number.to_string()),
        any_value::Value::StringValue(text) => decimal_text(text),
        _ => None,
    }
}

fn first<'a, T>(
    attributes: &Attributes<'a>,
    keys: impl IntoIterator<Item = &'a str>,
    read: impl Fn(&any_value::Value) -> Option<T>,
) -> Option<T> {
    keys.into_iter()
        .find_map(|key| attributes.get(key).and_then(|value| read(value)))
}

/// The span's tokens, or `None` when it carries no usage. The conventions count the cache in
/// `input_tokens`; a gateway whose cache exceeds it reported the uncached input alone.
pub fn tokens_of(span: &Attributes) -> Option<Tokens> {
    let input = first(span, INPUT_TOKENS, count_of);
    let output = first(span, OUTPUT_TOKENS, count_of);
    let cache_read = first(span, [CACHE_READ_TOKENS], count_of).unwrap_or(0);
    let cache_write = first(span, [CACHE_WRITE_TOKENS], count_of).unwrap_or(0);
    let reasoning = first(span, [REASONING_TOKENS], count_of).unwrap_or(0);
    if input.is_none() && output.is_none() && cache_read + cache_write == 0 {
        return None;
    }
    let mut input = input.unwrap_or(0);
    if cache_read + cache_write > input {
        input += cache_read + cache_write;
    }
    Some(Tokens::split(
        input,
        output.unwrap_or(0),
        cache_read,
        cache_write,
        reasoning,
    ))
}

fn is_tool_call(span: &Attributes) -> bool {
    span.contains_key("gen_ai.tool.call.id")
        || first(span, ["gen_ai.operation.name"], text_of).as_deref() == Some("execute_tool")
}

/// The call a span records, read with `settings`.
pub fn record_of(span: &Span, resource: &Attributes, settings: &Settings) -> Record {
    let attrs = attributes(&span.attributes);
    if is_tool_call(&attrs) {
        return Record::Ignored;
    }
    let Some(tokens) = tokens_of(&attrs) else {
        return Record::Ignored;
    };
    let request_id = match &settings.request_id_from {
        Some(keys) => first(&attrs, keys.iter().map(String::as_str), text_of),
        None => first(&attrs, DEFAULT_REQUEST_ID_FROM, text_of),
    }
    .unwrap_or_else(|| {
        format!(
            "{}-{}",
            hex::encode(&span.trace_id),
            hex::encode(&span.span_id)
        )
    });
    let customer_from: Vec<&str> = match &settings.customer_from {
        Some(keys) => keys.iter().map(String::as_str).collect(),
        None => DEFAULT_CUSTOMER_FROM.to_vec(),
    };
    let customer = customer_from.iter().find_map(|key| {
        first(&attrs, [*key], text_of).or_else(|| first(resource, [*key], text_of))
    });
    let Some(customer) = customer else {
        return Record::Rejected(format!(
            "Span {}: names no customer in {}",
            hex::encode(&span.span_id),
            customer_from.join(", ")
        ));
    };
    // Rounded to the microsecond events carry, as the LiteLLM endpoint rounds `endTime`: LiteLLM
    // exports `end_time.timestamp() * 1e9`, a few hundred nanoseconds off its microsecond.
    let ended_at = i64::try_from(span.end_time_unix_nano)
        .ok()
        .filter(|nanos| *nanos > 0)
        .and_then(|nanos| DateTime::from_timestamp_micros((nanos + 500) / 1_000));
    Record::Call(LlmCall {
        request_id,
        customer,
        model: first(&attrs, MODEL, text_of).unwrap_or_default(),
        provider: first(&attrs, PROVIDER, text_of).unwrap_or_default(),
        tokens,
        cost: first(&attrs, COST, number_text),
        payer: first(&attrs, [PAYER], text_of).or_else(|| first(resource, [PAYER], text_of)),
        ended_at,
    })
}

/// The records of a request, one per span, in order.
pub fn records_of(request: &ExportTraceServiceRequest, settings: &Settings) -> Vec<Record> {
    let mut records = Vec::new();
    for resource_spans in &request.resource_spans {
        let resource = resource_spans
            .resource
            .as_ref()
            .map(|resource| attributes(&resource.attributes))
            .unwrap_or_default();
        for scope_spans in &resource_spans.scope_spans {
            for span in &scope_spans.spans {
                records.push(record_of(span, &resource, settings));
            }
        }
    }
    records
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Protobuf,
    Json,
}

impl Encoding {
    fn of(headers: &HeaderMap) -> Result<Self, Problem> {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let media = content_type.split(';').next().unwrap_or_default().trim();
        match media.to_ascii_lowercase().as_str() {
            "application/x-protobuf" | "application/protobuf" => Ok(Encoding::Protobuf),
            "application/json" => Ok(Encoding::Json),
            _ => Err(Problem::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ProblemCode::UnsupportedMediaType,
                "OTLP/HTTP is `application/x-protobuf` or `application/json`",
            )),
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Encoding::Protobuf => "application/x-protobuf",
            Encoding::Json => "application/json",
        }
    }
}

fn decompressed(headers: &HeaderMap, body: Bytes) -> Result<Vec<u8>, Problem> {
    let encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("identity")
        .trim()
        .to_ascii_lowercase();
    match encoding.as_str() {
        "" | "identity" => Ok(body.to_vec()),
        "gzip" => {
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(body.as_ref())
                .take(MAX_DECODED_BYTES + 1)
                .read_to_end(&mut decoded)
                .map_err(|error| Problem::invalid_request(format!("Not gzip: {error}")))?;
            if decoded.len() as u64 > MAX_DECODED_BYTES {
                return Err(Problem::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    ProblemCode::PayloadTooLarge,
                    format!("The body is larger than {MAX_DECODED_BYTES} bytes decompressed"),
                ));
            }
            Ok(decoded)
        }
        other => Err(Problem::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ProblemCode::UnsupportedMediaType,
            format!("Content-Encoding `{other}` is not supported: send gzip or none"),
        )),
    }
}

pub fn decode(headers: &HeaderMap, body: Bytes) -> Result<ExportTraceServiceRequest, Problem> {
    let encoding = Encoding::of(headers)?;
    let body = decompressed(headers, body)?;
    match encoding {
        Encoding::Protobuf => ExportTraceServiceRequest::decode(body.as_slice())
            .map_err(|error| Problem::invalid_request(format!("Not OTLP protobuf: {error}"))),
        Encoding::Json => serde_json::from_slice(&body)
            .map_err(|error| Problem::invalid_request(format!("Not OTLP JSON: {error}"))),
    }
}

/// The partial success OTLP answers when some spans are rejected, naming the first ones.
pub fn partial_success(
    rejected: &std::collections::BTreeMap<usize, String>,
) -> Option<ExportTracePartialSuccess> {
    if rejected.is_empty() {
        return None;
    }
    let mut message: Vec<String> = rejected.values().take(MAX_REASONS).cloned().collect();
    if rejected.len() > MAX_REASONS {
        message.push(format!("and {} more", rejected.len() - MAX_REASONS));
    }
    Some(ExportTracePartialSuccess {
        rejected_spans: rejected.len() as i64,
        error_message: message.join("; "),
    })
}

/// The document's schema only: an OTLP `ExportTraceServiceRequest`.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct OtlpTraces(std::marker::PhantomData<Vec<u8>>);

/// Export OTLP traces
///
/// Bills the LLM calls in OpenTelemetry traces an AI gateway exports (OTLP/HTTP, `/v1/traces`
/// appended to the endpoint `/api/v2/events/otlp`): `application/x-protobuf` or
/// `application/json`, gzip or not, answered in the same encoding. A span with GenAI usage
/// attributes (`gen_ai.usage.input_tokens`, `output_tokens`, `cache_read.input_tokens`,
/// `cache_creation.input_tokens`, `reasoning.output_tokens`) is one call: one `llm_tokens`
/// event per token class with tokens, cached tokens out of `input` and reasoning out of
/// `output`, its `event_id` `<request id>:<class>`, so a batch sent again is billed once. Other
/// spans are ignored.
///
/// The request id is the first of `Meteroid-Request-Id-From` (default `litellm.call_id`,
/// `gen_ai.request.id`), else `<trace id>-<span id>`. The customer is `ext:<value>`, the
/// first of `Meteroid-Customer-From` found on the span, then on its resource (default
/// `meteroid.customer`, then LiteLLM's organization alias, end user and team alias).
/// `Meteroid-Event-Code` changes the event code. Spans naming no customer, or one Meteroid
/// does not know, are counted in `partialSuccess.rejectedSpans` and named in its
/// `errorMessage`; the others are billed.
#[utoipa::path(
    post,
    tag = "Events",
    path = "/api/v2/events/otlp/v1/traces",
    request_body(content(
        (OtlpTraces = "application/x-protobuf"),
        (OtlpTraces = "application/json"),
    )),
    responses(
        (status = 200, description = "An OTLP `ExportTraceServiceResponse`, its `partialSuccess` naming the spans rejected", content(
            (OtlpTraces = "application/x-protobuf"),
            (OtlpTraces = "application/json"),
        )),
        (status = 400, description = "The body is not OTLP traces", body = Problem, content_type = "application/problem+json"),
        (status = 415, description = "Neither protobuf nor JSON, or compressed otherwise than gzip", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests or events, retry later", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "Metering is unavailable, retry later", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn export_traces(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Problem> {
    let settings = Settings::of(&headers)?;
    let encoding = Encoding::of(&headers)?;
    let request = decode(&headers, body)?;
    let records = records_of(&request, &settings);
    let outcome = super::ingest(&state, &auth, records, &settings.event_code).await?;
    let response = ExportTraceServiceResponse {
        partial_success: partial_success(&outcome.rejected),
    };
    let body = match encoding {
        Encoding::Protobuf => response.encode_to_vec(),
        Encoding::Json => serde_json::to_vec(&response).map_err(|_| Problem::internal())?,
    };
    let mut answer = (StatusCode::OK, body).into_response();
    answer.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(encoding.content_type()),
    );
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::AnyValue;
    use opentelemetry_proto::tonic::resource::v1::Resource;
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans};
    use std::collections::BTreeMap;

    fn kv(key: &str, value: any_value::Value) -> KeyValue {
        KeyValue {
            key: key.to_owned(),
            value: Some(AnyValue { value: Some(value) }),
            ..Default::default()
        }
    }

    fn string(key: &str, value: &str) -> KeyValue {
        kv(key, any_value::Value::StringValue(value.to_owned()))
    }

    fn int(key: &str, value: i64) -> KeyValue {
        kv(key, any_value::Value::IntValue(value))
    }

    fn span(attributes: Vec<KeyValue>) -> Span {
        Span {
            trace_id: vec![0xab; 16],
            span_id: vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
            name: "chat gpt-4.1".to_owned(),
            end_time_unix_nano: 1_795_708_762_704_301_000,
            attributes,
            ..Default::default()
        }
    }

    fn defaults() -> Settings {
        Settings {
            event_code: "llm_tokens".to_owned(),
            customer_from: None,
            request_id_from: None,
        }
    }

    fn call(record: Record) -> LlmCall {
        match record {
            Record::Call(call) => call,
            other => panic!("not a call: {other:?}"),
        }
    }

    fn fixture(bytes: &'static [u8]) -> Vec<Record> {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-protobuf"),
        );
        let request = decode(&headers, Bytes::from_static(bytes)).unwrap();
        records_of(&request, &defaults())
    }

    #[test]
    fn litellm_spans_are_billed_like_its_callback_bills_the_call() {
        // LiteLLM 1.111.0, an OpenAI answer: prompt 1000 (200 cached), completion 300 (100 reasoning).
        for (bytes, cache_read) in [
            (
                include_bytes!("fixtures/litellm-1.111.0-otel-v2.binpb").as_slice(),
                200,
            ),
            (
                include_bytes!("fixtures/litellm-1.111.0-otel.binpb").as_slice(),
                0,
            ),
        ] {
            let records = fixture(bytes);
            let calls: Vec<LlmCall> = records
                .into_iter()
                .filter_map(|record| match record {
                    Record::Call(call) => Some(call),
                    Record::Ignored => None,
                    Record::Rejected(reason) => panic!("{reason}"),
                })
                .collect();
            assert_eq!(calls.len(), 1, "only the LLM call span carries usage");
            let call = &calls[0];
            assert_eq!(call.customer, "acme-gmbh");
            assert_eq!(call.model, "gpt-mock");
            assert_eq!(call.provider, "openai");
            assert_eq!(call.cost.as_deref(), Some("0"));
            assert_eq!(call.request_id.len(), 36, "litellm.call_id");
            // LiteLLM's spans carry no reasoning count: it is billed as output.
            assert_eq!(
                call.tokens,
                Tokens {
                    input: 1000 - cache_read,
                    output: 300,
                    cache_read,
                    cache_write: 0,
                    reasoning: 0
                }
            );
        }
    }

    #[test]
    fn a_genai_span_is_split_by_class() {
        let record = record_of(
            &span(vec![
                string("gen_ai.operation.name", "chat"),
                string("gen_ai.request.model", "claude-sonnet-4-5"),
                string("gen_ai.provider.name", "anthropic"),
                int("gen_ai.usage.input_tokens", 600),
                int("gen_ai.usage.output_tokens", 300),
                int("gen_ai.usage.cache_read.input_tokens", 300),
                int("gen_ai.usage.cache_creation.input_tokens", 200),
                int("gen_ai.usage.reasoning.output_tokens", 100),
                string("meteroid.customer", "acme"),
                kv("gen_ai.usage.cost", any_value::Value::DoubleValue(0.00189)),
            ]),
            &Attributes::new(),
            &defaults(),
        );
        let call = call(record);
        assert_eq!(
            call.tokens,
            Tokens {
                input: 100,
                output: 200,
                cache_read: 300,
                cache_write: 200,
                reasoning: 100
            }
        );
        assert_eq!(
            call.request_id,
            "abababababababababababababababab-0102030405060708"
        );
        assert_eq!(call.cost.as_deref(), Some("0.00189"));
        assert_eq!(call.model, "claude-sonnet-4-5");
        let ids: Vec<String> = call
            .events("llm_tokens")
            .into_iter()
            .map(|e| e.event_id)
            .collect();
        assert_eq!(ids.len(), 5);
        assert!(ids[0].ends_with("-0102030405060708:input"));
    }

    #[test]
    fn a_span_ends_at_the_microsecond_its_litellm_payload_names() {
        let mut exported = span(vec![
            int("gen_ai.usage.input_tokens", 10),
            string("meteroid.customer", "acme"),
        ]);
        exported.end_time_unix_nano = 1_795_741_844_926_260_992;
        let otlp = call(record_of(&exported, &Attributes::new(), &defaults()));
        let payload = serde_json::json!({
            "litellm_call_id": "call-1",
            "end_user": "acme",
            "prompt_tokens": 10,
            "endTime": 1_795_741_844.926_261,
        });
        let Record::Call(logged) =
            crate::v2::gateway::litellm::record_of(&payload, &["end_user".to_owned()])
        else {
            panic!("a call");
        };
        assert_eq!(
            otlp.events("llm_tokens")[0].timestamp.as_deref(),
            Some("2026-11-27T01:10:44.926261Z")
        );
        assert_eq!(otlp.ended_at, logged.ended_at);
    }

    #[test]
    fn an_input_count_below_the_cache_is_the_uncached_input() {
        // Envoy AI Gateway's native Anthropic route reports Anthropic's own `input_tokens`.
        let call = call(record_of(
            &span(vec![
                int("gen_ai.usage.input_tokens", 100),
                int("gen_ai.usage.output_tokens", 50),
                int("gen_ai.usage.cache_read.input_tokens", 300),
                int("gen_ai.usage.cache_creation.input_tokens", 200),
                string("meteroid.customer", "acme"),
            ]),
            &Attributes::new(),
            &defaults(),
        ));
        assert_eq!(
            (
                call.tokens.input,
                call.tokens.cache_read,
                call.tokens.cache_write
            ),
            (100, 300, 200)
        );
    }

    #[test]
    fn spans_without_usage_and_tool_calls_are_ignored() {
        let none = record_of(
            &span(vec![string("http.route", "/v1/chat/completions")]),
            &Attributes::new(),
            &defaults(),
        );
        assert_eq!(none, Record::Ignored);
        let tool = record_of(
            &span(vec![
                string("gen_ai.tool.call.id", "call_1"),
                int("gen_ai.usage.input_tokens", 10),
                string("meteroid.customer", "acme"),
            ]),
            &Attributes::new(),
            &defaults(),
        );
        assert_eq!(tool, Record::Ignored, "Kong repeats the call's usage on it");
    }

    #[test]
    fn the_customer_is_on_the_span_then_on_its_resource() {
        let resource_attributes = vec![string("meteroid.customer", "from-resource")];
        let resource = attributes(&resource_attributes);
        let usage = || int("gen_ai.usage.prompt_tokens", 10);
        assert_eq!(
            call(record_of(&span(vec![usage()]), &resource, &defaults())).customer,
            "from-resource"
        );
        assert_eq!(
            call(record_of(
                &span(vec![usage(), string("litellm.team.alias", "team-a")]),
                &resource,
                &defaults()
            ))
            .customer,
            "from-resource",
            "meteroid.customer comes first, wherever it is"
        );
        let Record::Rejected(reason) = record_of(
            &span(vec![usage(), string("meteroid.customer", " ")]),
            &Attributes::new(),
            &defaults(),
        ) else {
            panic!("a span naming no customer is rejected");
        };
        assert!(reason.contains("0102030405060708"), "{reason}");
    }

    #[test]
    fn the_attributes_read_are_configurable() {
        let settings = Settings {
            customer_from: Some(vec!["tenant.id".to_owned()]),
            request_id_from: Some(vec!["x.request".to_owned()]),
            ..defaults()
        };
        let call = call(record_of(
            &span(vec![
                // vLLM's attributes: the deprecated counts and its own request id.
                int("gen_ai.usage.prompt_tokens", 12),
                int("gen_ai.usage.completion_tokens", 3),
                string("gen_ai.request.id", "vllm-1"),
                string("x.request", "req-9"),
                int("tenant.id", 42),
                string("meteroid.customer", "ignored"),
            ]),
            &Attributes::new(),
            &settings,
        ));
        assert_eq!(
            (call.request_id.as_str(), call.customer.as_str()),
            ("req-9", "42")
        );
        assert_eq!((call.tokens.input, call.tokens.output), (12, 3));
    }

    fn request_of(spans: Vec<Span>) -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![string("service.name", "gateway")],
                    ..Default::default()
                }),
                scope_spans: vec![ScopeSpans {
                    spans,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn otlp_is_read_as_protobuf_or_json_gzipped_or_not() {
        let request = request_of(vec![span(vec![
            int("gen_ai.usage.input_tokens", 7),
            string("meteroid.customer", "acme"),
        ])]);
        let headers = |content_type: &'static str, gzip: bool| {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            if gzip {
                headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            }
            headers
        };
        let gzipped = |bytes: &[u8]| {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            std::io::Write::write_all(&mut encoder, bytes).unwrap();
            Bytes::from(encoder.finish().unwrap())
        };
        let protobuf = request.encode_to_vec();
        assert_eq!(
            decode(
                &headers("application/x-protobuf", false),
                protobuf.clone().into()
            )
            .unwrap(),
            request
        );
        assert_eq!(
            decode(&headers("application/x-protobuf", true), gzipped(&protobuf)).unwrap(),
            request
        );
        // OTLP/JSON: hex ids, 64-bit integers as strings.
        let json = serde_json::json!({"resourceSpans": [{
            "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "gateway"}}]},
            "scopeSpans": [{"spans": [{
                "traceId": "abababababababababababababababab",
                "spanId": "0102030405060708",
                "name": "chat gpt-4.1",
                "endTimeUnixNano": "1795708762704301000",
                "attributes": [
                    {"key": "gen_ai.usage.input_tokens", "value": {"intValue": "7"}},
                    {"key": "meteroid.customer", "value": {"stringValue": "acme"}}
                ]
            }]}]
        }]});
        let read = decode(
            &headers("application/json; charset=utf-8", false),
            serde_json::to_vec(&json).unwrap().into(),
        )
        .unwrap();
        assert_eq!(read, request);
        assert_eq!(
            decode(&headers("text/plain", false), Bytes::new())
                .unwrap_err()
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            decode(
                &headers("application/x-protobuf", false),
                Bytes::from_static(b"\xff\xff")
            )
            .unwrap_err()
            .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn rejected_spans_are_counted_and_the_first_named() {
        let rejected: BTreeMap<usize, String> = (0..25)
            .map(|index| (index, format!("span {index}")))
            .collect();
        let partial = partial_success(&rejected).unwrap();
        assert_eq!(partial.rejected_spans, 25);
        assert!(partial.error_message.starts_with("span 0; span 1;"));
        assert!(partial.error_message.ends_with("span 19; and 5 more"));
        assert!(partial_success(&BTreeMap::new()).is_none());
    }
}
