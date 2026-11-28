//! LiteLLM's built-in generic API logger (`callbacks: ["generic_api"]`): its
//! `StandardLoggingPayload`s, read as the Meteroid callback reads them, so both bill a call alike.
use super::{LlmCall, Record, Settings, Tokens, decimal_text};
use crate::AppState;
use crate::v2::problem::Problem;
use axum::extract::State;
use axum::{Extension, Json};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::HeaderMap;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

/// The callback's default identity sources, in order.
pub const DEFAULT_CUSTOMER_FROM: [&str; 3] = ["customer_org_alias", "end_user", "team_alias"];

fn get<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.get(key))
}

/// A count as the callback reads it: a number or its text, at least 0, else 0.
fn count(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| number.as_f64().filter(|n| *n > 0.0).map(|n| n as u64))
            .unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

fn first_count(values: &[Option<&Value>]) -> u64 {
    values
        .iter()
        .map(|value| count(*value))
        .find(|count| *count > 0)
        .unwrap_or(0)
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub fn token_counts(payload: &Value) -> Tokens {
    let usage = get(payload, &["metadata", "usage_object"]).unwrap_or(&Value::Null);
    Tokens::split(
        first_count(&[payload.get("prompt_tokens"), usage.get("prompt_tokens")]),
        first_count(&[
            payload.get("completion_tokens"),
            usage.get("completion_tokens"),
        ]),
        first_count(&[
            get(usage, &["prompt_tokens_details", "cached_tokens"]),
            usage.get("cache_read_input_tokens"),
        ]),
        count(usage.get("cache_creation_input_tokens")),
        count(get(
            usage,
            &["completion_tokens_details", "reasoning_tokens"],
        )),
    )
}

/// The first identity of `sources` the call carries: what the customer's alias is.
pub fn customer_of(payload: &Value, sources: &[String]) -> Option<String> {
    let metadata = |key: &str| text(get(payload, &["metadata", key]));
    sources.iter().find_map(|source| match source.as_str() {
        "customer_org_alias" => metadata("user_api_key_customer_org_alias"),
        "customer_member_id" => metadata("user_api_key_customer_member_id"),
        // LiteLLM logs the request's `user` as `end_user` for proxied calls only, not SDK ones.
        "end_user" => text(payload.get("end_user"))
            .or_else(|| metadata("user_api_key_end_user_id"))
            .or_else(|| text(get(payload, &["model_parameters", "user"]))),
        "team_id" => metadata("user_api_key_team_id"),
        "team_alias" => metadata("user_api_key_team_alias"),
        "user_id" => metadata("user_api_key_user_id"),
        "key_alias" => metadata("user_api_key_alias"),
        _ => None,
    })
}

fn ended_at(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let seconds = value?.as_f64()?;
    DateTime::from_timestamp_micros((seconds * 1_000_000.0).round() as i64)
}

/// The call one payload logs; a failed call is not billed.
pub fn record_of(payload: &Value, customer_from: &[String]) -> Record {
    if !payload.is_object() || payload.get("status").and_then(Value::as_str) == Some("failure") {
        return Record::Ignored;
    }
    // The payload's `id` can be the provider's response id, which repeats: the last resort.
    let Some(request_id) = text(payload.get("litellm_call_id")).or_else(|| text(payload.get("id")))
    else {
        return Record::Rejected("The payload has no litellm_call_id".to_owned());
    };
    let Some(customer) = customer_of(payload, customer_from) else {
        return Record::Rejected(format!(
            "Call {request_id} names no customer in {}",
            customer_from.join(", ")
        ));
    };
    let model = text(payload.get("model_group"))
        .or_else(|| text(payload.get("model")))
        .unwrap_or_default();
    Record::Call(LlmCall {
        request_id,
        customer,
        model,
        provider: text(payload.get("custom_llm_provider")).unwrap_or_default(),
        tokens: token_counts(payload),
        cost: match payload.get("response_cost") {
            Some(Value::Number(cost)) => decimal_text(&cost.to_string()),
            Some(Value::String(cost)) => decimal_text(cost),
            _ => None,
        },
        payer: text(get(payload, &["metadata", "requester_metadata", "payer"])),
        ended_at: ended_at(payload.get("endTime")),
    })
}

/// The payloads of a body: a JSON array (the logger's default), one object, or NDJSON.
pub fn payloads(body: &[u8]) -> Result<Vec<Value>, Problem> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(item @ Value::Object(_)) => Ok(vec![item]),
        Ok(_) => Err(Problem::invalid_request(
            "The body is an array of LiteLLM logging payloads, one payload, or NDJSON",
        )),
        Err(_) => body
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
            .map(|line| {
                serde_json::from_slice(line).map_err(|error| {
                    Problem::invalid_request(format!("A line is not a JSON payload: {error}"))
                })
            })
            .collect(),
    }
}

/// A payload not billed.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct LitellmRejection {
    /// Its position in the batch.
    pub index: usize,
    pub message: String,
}

/// What became of a batch of LiteLLM logging payloads.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct LitellmIngestResult {
    /// The calls billed, each one event per token class with tokens.
    pub calls: usize,
    /// The events accepted. A copy of an event already received counts: it is ignored when
    /// stored.
    pub events: usize,
    /// Payloads with no usage to bill: failed calls.
    pub ignored: usize,
    pub rejected: Vec<LitellmRejection>,
}

/// Ingest LiteLLM logs
///
/// Bills the calls LiteLLM's generic API logger (`callbacks: ["generic_api"]`) sends: a JSON
/// array of `StandardLoggingPayload`s (its `json_array` format), one payload (`single`) or
/// NDJSON. Each successful call becomes one `llm_tokens` event per token class with tokens
/// (`input`, `output`, `cache_read`, `cache_write`, `reasoning`), its `event_id`
/// `<litellm_call_id>:<class>`: a batch sent again is billed once. Failed calls are ignored.
///
/// The customer is `ext:<identity>`, the first identity found among `Meteroid-Customer-From`
/// (default `customer_org_alias,end_user,team_alias`; also `customer_member_id`, `team_id`,
/// `user_id`, `key_alias`). `Meteroid-Event-Code` changes the event code. A payload naming no
/// customer, or one Meteroid does not know, is in `rejected`; the others are billed.
#[utoipa::path(
    post,
    tag = "Events",
    path = "/api/v2/events/litellm",
    request_body(content = Vec<Object>, content_type = "application/json"),
    responses(
        (status = 200, description = "The calls billed, and the payloads rejected", body = LitellmIngestResult),
        (status = 400, description = "The body is not LiteLLM logging payloads", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests or events, retry later", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "Metering is unavailable, retry later", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn ingest_litellm_logs(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<LitellmIngestResult>, Problem> {
    let settings = Settings::of(&headers)?;
    let customer_from = settings.customer_from.unwrap_or_else(|| {
        DEFAULT_CUSTOMER_FROM
            .iter()
            .map(|source| (*source).to_owned())
            .collect()
    });
    let records: Vec<Record> = payloads(&body)?
        .iter()
        .map(|payload| record_of(payload, &customer_from))
        .collect();
    let ignored = records
        .iter()
        .filter(|record| matches!(record, Record::Ignored))
        .count();
    let outcome = super::ingest(&state, &auth, records, &settings.event_code).await?;
    Ok(Json(LitellmIngestResult {
        calls: outcome.calls,
        events: outcome.events,
        ignored,
        rejected: outcome
            .rejected
            .into_iter()
            .map(|(index, message)| LitellmRejection { index, message })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("fixtures/litellm-1.111.0-usage.json")).unwrap()
    }

    fn defaults() -> Vec<String> {
        DEFAULT_CUSTOMER_FROM.map(str::to_owned).to_vec()
    }

    #[test]
    fn every_token_is_counted_once_in_its_class() {
        let fixtures = fixtures();
        assert_eq!(fixtures["litellm_version"], "1.111.0");
        // Anthropic and Bedrock: input 100, cache write 200, cache read 300, which LiteLLM sums.
        let cached = Tokens {
            input: 100,
            output: 50,
            cache_read: 300,
            cache_write: 200,
            reasoning: 0,
        };
        for (provider, tokens) in [
            ("anthropic", cached),
            ("bedrock", cached),
            (
                "openai",
                Tokens {
                    input: 800,
                    output: 200,
                    cache_read: 200,
                    cache_write: 0,
                    reasoning: 100,
                },
            ),
        ] {
            let payload = &fixtures["payloads"][provider];
            assert_eq!(token_counts(payload), tokens, "{provider}");
            let total: u64 = tokens.by_class().iter().map(|(_, count)| count).sum();
            assert_eq!(
                total,
                count(payload.get("prompt_tokens")) + count(payload.get("completion_tokens"))
            );
        }
    }

    #[test]
    fn a_payload_is_the_events_the_callback_sends() {
        let mut payload = fixtures()["payloads"]["anthropic"].clone();
        payload["end_user"] = json!("acme-gmbh");
        let Record::Call(call) = record_of(&payload, &["end_user".to_owned()]) else {
            panic!("a call");
        };
        let events = call.events("llm_tokens");
        let classes: Vec<&Value> = events
            .iter()
            .map(|e| &e.properties["token_class"])
            .collect();
        assert_eq!(classes, ["input", "output", "cache_read", "cache_write"]);
        assert_eq!(
            events[0].event_id,
            format!("{}:input", payload["litellm_call_id"].as_str().unwrap())
        );
        assert_eq!(events[0].customer, "ext:acme-gmbh");
        assert_eq!(
            events[0].properties["model"], "claude-sonnet-4-5",
            "an empty model group falls back to the model"
        );
        assert_eq!(events[0].properties["provider"], "anthropic");
        assert_eq!(events[0].properties["cost_reported"], "0.00189");
        assert_eq!(events[0].properties["cost_currency"], "USD");
        assert_eq!(events[0].properties["tokens"], "100");
        assert_eq!(
            events[0].timestamp.as_deref(),
            Some("2026-11-26T15:59:22.704301Z")
        );
        assert!(!events[1].properties.contains_key("cost_reported"));
    }

    fn as_json(events: Vec<crate::v2::events::IngestEvent>) -> Value {
        events
            .into_iter()
            .map(|event| {
                json!({
                    "event_id": event.event_id,
                    "code": event.code,
                    "customer": event.customer,
                    "properties": event.properties,
                    "timestamp": event.timestamp,
                })
            })
            .collect()
    }

    #[test]
    fn the_events_are_those_the_removed_callback_sent() {
        let golden: Value =
            serde_json::from_str(include_str!("fixtures/litellm-callback-events.json")).unwrap();
        let fixtures = fixtures();
        let generic: Value =
            serde_json::from_str(include_str!("fixtures/litellm-1.111.0-generic-api.json"))
                .unwrap();
        for (name, expected) in golden["events"].as_object().unwrap() {
            let payload = if name == "generic_api" {
                generic[0].clone()
            } else {
                let mut payload = fixtures["payloads"][name].clone();
                payload["end_user"] = json!("acme-gmbh");
                payload["metadata"]["requester_metadata"] = json!({"payer": "team-a"});
                payload
            };
            let Record::Call(call) = record_of(&payload, &defaults()) else {
                panic!("{name}: a call");
            };
            assert_eq!(&as_json(call.events("llm_tokens")), expected, "{name}");
        }
    }

    #[test]
    fn the_customer_is_the_first_identity_found() {
        let payload = json!({
            "end_user": "  ",
            "metadata": {
                "user_api_key_end_user_id": "end-user",
                "user_api_key_team_alias": "team",
                "user_api_key_customer_org_alias": ""
            }
        });
        assert_eq!(
            customer_of(&payload, &defaults()).as_deref(),
            Some("end-user")
        );
        assert_eq!(
            customer_of(&payload, &["key_alias".to_owned(), "team_alias".to_owned()]).as_deref(),
            Some("team")
        );
        assert_eq!(customer_of(&payload, &["user_id".to_owned()]), None);
    }

    #[test]
    fn an_sdk_call_names_its_customer_by_the_request_s_user() {
        let sdk = json!({"end_user": null, "model_parameters": {"user": "acme", "max_tokens": 5}});
        let proxied = json!({"end_user": "acme", "model_parameters": {"user": "upstream-id"}});
        for payload in [sdk, proxied] {
            assert_eq!(customer_of(&payload, &defaults()).as_deref(), Some("acme"));
        }
    }

    #[test]
    fn the_generic_loggers_batch_is_read_as_it_sends_it() {
        // LiteLLM 1.111.0's generic API logger, an OpenAI answer: 1000 in (200 cached), 300 out
        // (100 reasoning), `user` acme-gmbh.
        let body = include_bytes!("fixtures/litellm-1.111.0-generic-api.json");
        let payloads = payloads(body).unwrap();
        assert_eq!(payloads.len(), 1);
        let Record::Call(call) = record_of(&payloads[0], &defaults()) else {
            panic!("a call");
        };
        assert_eq!(
            (call.customer.as_str(), call.model.as_str()),
            ("acme-gmbh", "gpt-mock")
        );
        assert_eq!(
            call.tokens,
            Tokens {
                input: 800,
                output: 200,
                cache_read: 200,
                cache_write: 0,
                reasoning: 100
            }
        );
        let ndjson = payloads
            .iter()
            .map(|payload| serde_json::to_string(payload).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(super::payloads(ndjson.as_bytes()).unwrap(), payloads);
        assert_eq!(
            super::payloads(serde_json::to_string(&payloads[0]).unwrap().as_bytes()).unwrap(),
            payloads
        );
        assert!(super::payloads(b"42").is_err());
    }

    #[test]
    fn failed_calls_are_ignored_and_unattributed_ones_rejected() {
        let failed = json!({"status": "failure", "litellm_call_id": "c", "end_user": "acme"});
        assert_eq!(record_of(&failed, &defaults()), Record::Ignored);
        let anonymous = json!({"status": "success", "litellm_call_id": "c", "prompt_tokens": 3});
        assert!(matches!(
            record_of(&anonymous, &defaults()),
            Record::Rejected(reason) if reason.contains("Call c names no customer")
        ));
        let unnamed = json!({"status": "success", "end_user": "acme"});
        assert!(matches!(
            record_of(&unnamed, &defaults()),
            Record::Rejected(_)
        ));
    }
}
