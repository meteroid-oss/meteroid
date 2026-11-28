//! LLM usage AI gateways export (OTLP GenAI spans, LiteLLM's generic logger) as the LLM usage
//! convention's events: one per token class of a call, its id `<request id>:<class>`, so a batch
//! sent again is billed once. `docs/ai-gateways/README.md` sets each gateway up.
pub mod litellm;
pub mod otlp;

use crate::AppState;
use crate::v2::events::{IngestEvent, MAX_EVENTS, metering_event};
use crate::v2::problem::Problem;
use chrono::{DateTime, Utc};
use common_domain::identifiers::validate_code;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::HeaderMap;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(otlp::export_traces))
        .routes(routes!(litellm::ingest_litellm_logs))
}

/// The paths gateways send to, whose batches may be larger than an API call's.
pub const PATHS: [&str; 2] = ["/api/v2/events/otlp/v1/traces", "/api/v2/events/litellm"];
/// The largest batch a gateway may send.
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

pub const DEFAULT_EVENT_CODE: &str = "llm_tokens";
pub const EVENT_CODE_HEADER: &str = "meteroid-event-code";
pub const CUSTOMER_FROM_HEADER: &str = "meteroid-customer-from";
pub const REQUEST_ID_FROM_HEADER: &str = "meteroid-request-id-from";
/// The currency gateways report their cost in.
pub const REPORTED_CURRENCY: &str = "USD";
/// Metering takes events up to 24 hours old without backfilling.
const BACKFILL_AFTER_HOURS: i64 = 23;

/// A call's tokens by class, each counted once: cached and cache-written tokens are out of
/// `input`, reasoning tokens out of `output`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl Tokens {
    /// From totals that include the cache (`input`) and the reasoning (`output`).
    pub fn split(
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        reasoning: u64,
    ) -> Self {
        let cache_read = cache_read.min(input);
        let cache_write = cache_write.min(input - cache_read);
        let reasoning = reasoning.min(output);
        Tokens {
            input: input - cache_read - cache_write,
            output: output - reasoning,
            cache_read,
            cache_write,
            reasoning,
        }
    }

    pub fn by_class(&self) -> [(&'static str, u64); 5] {
        [
            ("input", self.input),
            ("output", self.output),
            ("cache_read", self.cache_read),
            ("cache_write", self.cache_write),
            ("reasoning", self.reasoning),
        ]
    }
}

/// One LLM call of a gateway's export, as the convention names it.
#[derive(Clone, Debug, PartialEq)]
pub struct LlmCall {
    pub request_id: String,
    /// The customer's alias, sent as `ext:<alias>`.
    pub customer: String,
    pub model: String,
    pub provider: String,
    pub tokens: Tokens,
    /// What the gateway computed the call cost, in USD, as plain decimal text.
    pub cost: Option<String>,
    pub payer: Option<String>,
    pub ended_at: Option<DateTime<Utc>>,
}

impl LlmCall {
    /// The call's events, the cost on the first only: the margin spreads it over the classes.
    pub fn events(&self, event_code: &str) -> Vec<IngestEvent> {
        let mut events: Vec<IngestEvent> = Vec::new();
        for (class, tokens) in self.tokens.by_class() {
            if tokens == 0 {
                continue;
            }
            let mut properties = BTreeMap::from([
                ("model", self.model.clone()),
                ("provider", self.provider.clone()),
                ("token_class", class.to_owned()),
                ("request_id", self.request_id.clone()),
                ("tokens", tokens.to_string()),
            ]);
            if let (true, Some(cost)) = (events.is_empty(), &self.cost) {
                properties.insert("cost_reported", cost.clone());
                properties.insert("cost_currency", REPORTED_CURRENCY.to_owned());
            }
            if let Some(payer) = &self.payer {
                properties.insert("payer", payer.clone());
            }
            events.push(IngestEvent {
                event_id: format!("{}:{class}", self.request_id),
                code: event_code.to_owned(),
                customer: format!("ext:{}", self.customer),
                timestamp: self
                    .ended_at
                    .map(|at| at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()),
                properties: properties
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), serde_json::Value::String(value)))
                    .collect(),
            });
        }
        events
    }
}

/// What one record of a gateway's batch is.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Call(LlmCall),
    /// No LLM usage in it (a gateway's other spans, a failed call).
    Ignored,
    /// LLM usage that cannot be billed, and why.
    Rejected(String),
}

/// A number as plain decimal text (`0.00001`, never `1e-5`), which metering parses.
pub fn decimal_text(text: &str) -> Option<String> {
    let text = text.trim();
    let number = if text.contains(['e', 'E']) {
        Decimal::from_scientific(text).ok()?
    } else {
        Decimal::from_str(text).ok()?
    };
    Some(number.normalize().to_string())
}

/// How a request asks its usage to be read: from the `Meteroid-*` headers its gateway sends.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub event_code: String,
    pub customer_from: Option<Vec<String>>,
    pub request_id_from: Option<Vec<String>>,
}

fn header_list(headers: &HeaderMap, name: &str) -> Result<Option<Vec<String>>, Problem> {
    let Some(value) = headers.get(name) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| Problem::invalid_request(format!("`{name}` is not text")))?;
    let list: Vec<String> = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect();
    if list.is_empty() {
        return Err(Problem::invalid_request(format!(
            "`{name}` lists no attribute"
        )));
    }
    Ok(Some(list))
}

impl Settings {
    pub fn of(headers: &HeaderMap) -> Result<Self, Problem> {
        let event_code = match headers.get(EVENT_CODE_HEADER) {
            None => DEFAULT_EVENT_CODE.to_owned(),
            Some(value) => {
                let code = value.to_str().unwrap_or_default().trim().to_owned();
                validate_code(&code).map_err(|error| {
                    Problem::invalid_request(format!("`{EVENT_CODE_HEADER}`: {error}"))
                })?;
                code
            }
        };
        Ok(Settings {
            event_code,
            customer_from: header_list(headers, CUSTOMER_FROM_HEADER)?,
            request_id_from: header_list(headers, REQUEST_ID_FROM_HEADER)?,
        })
    }
}

/// What became of a batch's records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The calls none of whose events is rejected.
    pub calls: usize,
    pub events: usize,
    /// The records rejected, by position in the batch, with the first reason.
    pub rejected: BTreeMap<usize, String>,
}

/// Ingests the calls of `records` through v2's ingest, partial failures allowed: a record is
/// rejected when one of its events is. Metering deduplicates the events of a batch sent again.
pub async fn ingest(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    records: Vec<Record>,
    event_code: &str,
) -> Result<Outcome, Problem> {
    let mut outcome = Outcome::default();
    let mut calls = Vec::new();
    let mut forwarded = Vec::new();
    let mut owners: HashMap<String, usize> = HashMap::new();
    let backfill_before = common_clock::now() - chrono::Duration::hours(BACKFILL_AFTER_HOURS);
    let mut backfilling = false;
    for (index, record) in records.into_iter().enumerate() {
        let call = match record {
            Record::Call(call) => call,
            Record::Ignored => continue,
            Record::Rejected(reason) => {
                outcome.rejected.insert(index, reason);
                continue;
            }
        };
        calls.push(index);
        backfilling |= call.ended_at.is_some_and(|at| at < backfill_before);
        for event in call.events(event_code) {
            match metering_event(event, index) {
                Ok(event) => {
                    owners.entry(event.id.clone()).or_insert(index);
                    forwarded.push(event);
                }
                Err(rejection) => {
                    let reason = format!("{}: {}", rejection.event_id, rejection.message);
                    outcome.rejected.entry(index).or_insert(reason);
                }
            }
        }
    }
    for chunk in forwarded.chunks(MAX_EVENTS) {
        let result =
            crate::events::router::ingest(state, auth, chunk.to_vec(), backfilling, false).await?;
        outcome.events += chunk.len() - result.failures.len();
        for failure in result.failures {
            if let Some(index) = owners.get(&failure.event_id) {
                let reason = format!("{}: {}", failure.event_id, failure.reason);
                outcome.rejected.entry(*index).or_insert(reason);
            }
        }
    }
    outcome.calls = calls
        .iter()
        .filter(|index| !outcome.rejected.contains_key(index))
        .count();
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_counted_once_in_their_class() {
        assert_eq!(
            Tokens::split(1000, 300, 200, 0, 100),
            Tokens {
                input: 800,
                output: 200,
                cache_read: 200,
                cache_write: 0,
                reasoning: 100
            }
        );
        let clamped = Tokens::split(10, 5, 30, 30, 9);
        assert_eq!(
            (clamped.input, clamped.cache_read, clamped.cache_write),
            (0, 10, 0)
        );
        assert_eq!((clamped.output, clamped.reasoning), (0, 5));
    }

    #[test]
    fn decimals_are_plain_text() {
        assert_eq!(decimal_text("1e-5").as_deref(), Some("0.00001"));
        assert_eq!(decimal_text("0.001890").as_deref(), Some("0.00189"));
        assert_eq!(decimal_text("0").as_deref(), Some("0"));
        assert_eq!(decimal_text("12").as_deref(), Some("12"));
        assert_eq!(decimal_text("NaN"), None);
    }

    #[test]
    fn a_call_is_one_event_per_class_with_tokens_its_cost_on_the_first() {
        let call = LlmCall {
            request_id: "call-1".to_owned(),
            customer: "acme".to_owned(),
            model: "gpt-4.1".to_owned(),
            provider: "openai".to_owned(),
            tokens: Tokens::split(1000, 300, 200, 0, 100),
            cost: Some("0.0042".to_owned()),
            payer: Some("team-a".to_owned()),
            ended_at: DateTime::from_timestamp(1_795_708_762, 704_301_000),
        };
        let events = call.events("llm_tokens");
        let ids: Vec<&str> = events.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "call-1:input",
                "call-1:output",
                "call-1:cache_read",
                "call-1:reasoning"
            ]
        );
        assert_eq!(events[0].customer, "ext:acme");
        assert_eq!(
            events[0].timestamp.as_deref(),
            Some("2026-11-26T15:59:22.704301Z")
        );
        assert_eq!(events[0].properties["cost_reported"], "0.0042");
        assert_eq!(events[0].properties["cost_currency"], "USD");
        assert!(!events[1].properties.contains_key("cost_reported"));
        assert_eq!(events[3].properties["tokens"], "100");
        assert_eq!(events[3].properties["payer"], "team-a");
    }
}
