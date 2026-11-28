//! `POST /api/v2/events/ingest` (ADR-0013 §13): v2's own body and result, ingested by the
//! metering path v1's handler uses, with its limits (the metering request rate, the events per
//! second). An event names its `customer` as v2 does everywhere, its properties are JSON
//! scalars, and the result names each rejected event at its pointer.
use crate::AppState;
use crate::v2::extract::Body;
use crate::v2::ids::{EXTERNAL_PREFIX, parse_id};
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use axum::extract::State;
use axum::{Extension, Json};
use common_domain::identifiers::validate_code;
use common_domain::ids::CustomerId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use metering_grpc::meteroid::metering::v1 as grpc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(ingest_events))
}

/// Events in one request, at most: metering's own limit.
pub const MAX_EVENTS: usize = 500;
const MAX_EVENT_ID_CHARS: usize = 255;

/// A property's value: a string, a number or a boolean. Metering keeps every value as text: a
/// number as written (`1048576`, `0.25`; send a string to keep a value past 2^64 or 17
/// significant digits exact), a boolean as `true` or `false`. `null`, an object or an array is
/// refused.
// The document's schema only: a body reads its values as JSON and checks them (`property_text`).
#[derive(ToSchema)]
#[serde(untagged)]
pub enum EventPropertyValue {
    String(String),
    Number(f64),
    Boolean(bool),
}

/// One usage event.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct IngestEvent {
    /// 1 to 255 characters; a UUID or ULID is recommended. An event is deduplicated by its
    /// `event_id` and `customer`: the first copy received is kept, later copies are ignored.
    pub event_id: String,
    /// The event code (`api_calls`): every metric of this `event_code` reads the event.
    pub code: String,
    /// The customer: its Meteroid id, or its alias as `ext:<alias>`.
    #[schema(example = "ext:acme")]
    pub customer: String,
    /// RFC 3339; the time it is received when left out. Between 24 hours ago and 1 hour from
    /// now, or up to 13 months ago with `allow_backfilling`.
    #[schema(example = "2026-01-15T10:30:00Z", format = DateTime)]
    #[serde(default)]
    pub timestamp: Option<String>,
    /// What metrics filter, group and aggregate by.
    #[schema(
        value_type = HashMap<String, EventPropertyValue>,
        example = json!({"region": "us-east-1", "tier": "pro", "bytes": 1048576})
    )]
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// Events to ingest, 1 to 500 (the body 1 MB at most).
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct IngestEvents {
    /// The events, 1 to 500.
    pub events: Vec<IngestEvent>,
    /// Accept timestamps up to 13 months in the past, not 24 hours. `false` by default.
    #[serde(default)]
    pub allow_backfilling: bool,
    /// Ingest the accepted events of a batch some events are rejected from, and answer which
    /// were. `false` by default: one rejected event refuses the whole batch (`422`).
    #[serde(default)]
    pub allow_partial_failures: bool,
}

/// An event a batch was ingested without.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct IngestRejection {
    /// Its position in `events`.
    pub index: usize,
    /// A JSON pointer to what is refused: the event (`/events/3`) or one of its fields
    /// (`/events/3/customer`).
    pub path: String,
    /// The `event_id` it was sent with.
    pub event_id: String,
    /// Why it is rejected.
    pub message: String,
}

/// What became of a batch: every event not in `rejected` is accepted.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct IngestEventsResult {
    /// How many events were accepted. A copy of an event already received counts: it is
    /// ignored when stored.
    pub accepted: usize,
    /// The events rejected, in the batch's order; empty when none is.
    pub rejected: Vec<IngestRejection>,
}

/// A property's value as metering keeps it, or why it is refused.
fn property_text(value: serde_json::Value) -> Result<String, &'static str> {
    match value {
        serde_json::Value::String(text) => Ok(text),
        serde_json::Value::Number(number) => Ok(number.to_string()),
        serde_json::Value::Bool(flag) => Ok(flag.to_string()),
        serde_json::Value::Null => Err("A property is a string, a number or a boolean, not null"),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Err("A property is a string, a number or a boolean")
        }
    }
}

/// An event's properties as metering keeps them; the first refused, at its pointer under `path`.
pub fn event_properties(
    properties: HashMap<String, serde_json::Value>,
    path: &str,
) -> Result<HashMap<String, String>, FieldError> {
    properties
        .into_iter()
        .map(|(key, value)| match property_text(value) {
            Ok(text) => Ok((key, text)),
            Err(message) => Err(FieldError {
                path: format!("{path}/{}", pointer_token(&key)),
                code: "invalid".to_owned(),
                message: message.to_owned(),
            }),
        })
        .collect()
}

/// `key` as a JSON pointer's reference token (RFC 6901).
fn pointer_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// An event's customer as metering is handed it: `ext:<alias>` its alias (whatever the alias
/// looks like), else a Meteroid id.
fn event_customer(raw: &str) -> Result<grpc::event::CustomerId, &'static str> {
    if let Some(alias) = raw.strip_prefix(EXTERNAL_PREFIX) {
        return if alias.trim().is_empty() {
            Err("`ext:` is followed by the customer's alias")
        } else {
            Ok(grpc::event::CustomerId::ExternalCustomerAlias(
                alias.to_owned(),
            ))
        };
    }
    parse_id::<CustomerId>(raw.trim())
        .map(|id| grpc::event::CustomerId::MeteroidCustomerId(id.to_string()))
        .ok_or("Name the customer by its id, or by its alias as `ext:<alias>`")
}

/// The event metering is handed, or why it is refused, at a pointer under `/events/<index>`.
pub fn metering_event(event: IngestEvent, index: usize) -> Result<grpc::Event, IngestRejection> {
    let IngestEvent {
        event_id,
        code,
        customer,
        timestamp,
        properties,
    } = event;
    let refused = |path: String, message: String| IngestRejection {
        index,
        path,
        event_id: event_id.clone(),
        message,
    };
    let at = format!("/events/{index}");
    let id_chars = event_id.chars().count();
    if id_chars == 0 || id_chars > MAX_EVENT_ID_CHARS {
        return Err(refused(
            format!("{at}/event_id"),
            format!("An event_id is 1 to {MAX_EVENT_ID_CHARS} characters"),
        ));
    }
    validate_code(&code).map_err(|error| refused(format!("{at}/code"), error.to_string()))?;
    let customer = event_customer(&customer)
        .map_err(|message| refused(format!("{at}/customer"), message.to_owned()))?;
    let properties = event_properties(properties, &format!("{at}/properties"))
        .map_err(|error| refused(error.path, error.message))?;
    Ok(grpc::Event {
        id: event_id,
        code,
        customer_id: Some(customer),
        timestamp: timestamp.unwrap_or_default(),
        properties,
    })
}

/// Ingest events
///
/// Ingests usage events for metering and billing, 1 to 500 a request (the body 1 MB at most).
/// An event names its `customer` by Meteroid id or as `ext:<alias>`; its `properties` are
/// strings, numbers or booleans, which metering keeps as text (`1048576`, `true`). Its
/// `timestamp` (RFC 3339, the time it is received when left out) is between 24 hours ago and
/// 1 hour from now, or up to 13 months ago with `allow_backfilling`.
///
/// Events are deduplicated by `(event_id, customer)`: re-sending the same pair is not counted
/// twice. The first copy received is kept and later copies are ignored, whatever they change.
///
/// By default one rejected event refuses the whole batch: the answer is `422
/// validation_failed`, each rejected event in `errors` at its pointer (`/events/3`,
/// `/events/3/customer`); sending the batch again once fixed is safe. With
/// `allow_partial_failures`, the accepted events are ingested and the `200` names the rejected
/// ones in `rejected`. Metering names a rejected event by its `event_id` only: an `event_id`
/// sent twice in one batch is matched to its copies in the batch's order.
#[utoipa::path(
    post,
    tag = "Events",
    path = "/api/v2/events/ingest",
    request_body(content = IngestEvents, example = json!(crate::v2::examples::request("ingest_events"))),
    responses(
        (status = 200, description = "The events accepted, and those rejected with `allow_partial_failures`", body = IngestEventsResult, example = json!(crate::v2::examples::response("ingest_events"))),
        (status = 422, description = "An event is rejected (without `allow_partial_failures`, the batch is), or the batch is empty or too large", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests or events, retry later", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "Metering is unavailable, retry later", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn ingest_events(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(request): Body<IngestEvents>,
) -> Result<Json<IngestEventsResult>, Problem> {
    let total = request.events.len();
    if total == 0 || total > MAX_EVENTS {
        return Err(Problem::field(
            "/events",
            "invalid",
            format!("A batch has 1 to {MAX_EVENTS} events; this one has {total}"),
        ));
    }
    let partial = request.allow_partial_failures;
    let mut rejected = Vec::new();
    let mut forwarded = Vec::with_capacity(total);
    // Where each forwarded event is in the batch, by `event_id`: metering answers no position.
    let mut positions: HashMap<String, VecDeque<usize>> = HashMap::new();
    for (index, event) in request.events.into_iter().enumerate() {
        match metering_event(event, index) {
            Ok(event) => {
                positions
                    .entry(event.id.clone())
                    .or_default()
                    .push_back(index);
                forwarded.push(event);
            }
            Err(rejection) => rejected.push(rejection),
        }
    }
    // A batch refused whole before metering is not sent: metering would ingest what it accepts.
    if !forwarded.is_empty() && (partial || rejected.is_empty()) {
        let result = crate::events::router::ingest(
            &state,
            &auth,
            forwarded,
            request.allow_backfilling,
            !partial,
        )
        .await
        .map_err(Problem::from)?;
        rejected.extend(result.failures.into_iter().map(|failure| {
            let index = positions
                .get_mut(&failure.event_id)
                .and_then(VecDeque::pop_front);
            IngestRejection {
                index: index.unwrap_or(total),
                path: index
                    .map(|index| format!("/events/{index}"))
                    .unwrap_or_else(|| "/events".to_owned()),
                event_id: failure.event_id,
                message: failure.reason,
            }
        }));
        rejected.sort_by_key(|rejection| rejection.index);
    }
    if !partial && !rejected.is_empty() {
        return Err(refused_whole(&rejected));
    }
    Ok(Json(IngestEventsResult {
        accepted: total - rejected.len(),
        rejected,
    }))
}

/// A batch refused whole (`allow_partial_failures` false, some event rejected) is `422
/// validation_failed`, each rejection at its pointer. Sending the batch again is safe whatever
/// reached metering: events are deduplicated by `(event_id, customer)`.
fn refused_whole(rejected: &[IngestRejection]) -> Problem {
    let mut problem = Problem::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        ProblemCode::ValidationFailed,
        format!(
            "The batch is refused: {} of its events are rejected. Fix them and send the batch \
             again (events are deduplicated), or set allow_partial_failures to ingest the others",
            rejected.len()
        ),
    );
    problem.errors = rejected
        .iter()
        .map(|rejection| FieldError {
            path: rejection.path.clone(),
            code: "invalid".to_owned(),
            message: format!("{}: {}", rejection.event_id, rejection.message),
        })
        .collect();
    problem
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;
    use serde_json::json;

    fn event(body: serde_json::Value) -> IngestEvent {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn properties_are_kept_as_text_and_null_refused() {
        let kept = event_properties(
            serde_json::from_value(json!({
                "region": "eu", "bytes": 1048576, "ratio": 0.25, "delta": -12, "beta": true
            }))
            .unwrap(),
            "/events/0/properties",
        )
        .unwrap();
        assert_eq!(kept["region"], "eu");
        assert_eq!(kept["bytes"], "1048576");
        assert_eq!(kept["ratio"], "0.25");
        assert_eq!(kept["delta"], "-12");
        assert_eq!(kept["beta"], "true");

        for value in [json!(null), json!([1]), json!({})] {
            let refused = event_properties(
                HashMap::from([("a/b".to_owned(), value)]),
                "/events/2/properties",
            )
            .unwrap_err();
            assert_eq!(refused.path, "/events/2/properties/a~1b");
        }
    }

    #[test]
    fn a_customer_is_an_id_or_an_ext_alias() {
        let id = CustomerId::new();
        assert_eq!(
            event_customer(&id.to_string()).unwrap(),
            grpc::event::CustomerId::MeteroidCustomerId(id.to_string())
        );
        assert_eq!(
            event_customer(&format!("ext:{id}")).unwrap(),
            grpc::event::CustomerId::ExternalCustomerAlias(id.to_string()),
            "an alias shaped like an id is an alias"
        );
        assert!(event_customer("acme").is_err(), "a bare alias");
        assert!(event_customer("ext: ").is_err());
    }

    #[test]
    fn an_event_is_refused_at_its_field() {
        let refused = metering_event(
            event(json!({"event_id": "e1", "code": "api_calls", "customer": "acme"})),
            4,
        )
        .unwrap_err();
        assert_eq!(refused.path, "/events/4/customer");
        assert_eq!(refused.event_id, "e1");
        let unnamed = metering_event(
            event(json!({"event_id": "", "code": "api_calls", "customer": "ext:acme"})),
            0,
        )
        .unwrap_err();
        assert_eq!(unnamed.path, "/events/0/event_id");
        assert!(
            serde_json::from_value::<IngestEvent>(
                json!({"event_id": "e", "code": "c", "customer_id": "ext:acme"})
            )
            .is_err(),
            "v1's customer_id is not v2's"
        );
    }
}
