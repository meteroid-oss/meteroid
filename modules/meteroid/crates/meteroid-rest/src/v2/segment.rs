//! `POST /api/v2/events/segment`: Segment's `track` events, as its Webhooks (Actions) destination
//! sends them, ingested by the same metering path as `/api/v2/events/ingest`. Partial failures
//! are always on: one event Meteroid refuses never makes Segment retry the batch.
use crate::AppState;
use crate::v2::events::{IngestEventsResult, IngestRejection, MAX_EVENTS};
use crate::v2::extract::{Body, Query};
use crate::v2::problem::Problem;
use axum::extract::State;
use axum::{Extension, Json};
use common_domain::identifiers::validate_code;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use metering_grpc::meteroid::metering::v1 as grpc;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(ingest_segment_events))
}

/// Segment's largest batch.
pub const MAX_SEGMENT_EVENTS: usize = 4000;
const MAX_MESSAGE_ID_CHARS: usize = 255;

/// Which Segment identity names the Meteroid customer, by its alias.
#[derive(Deserialize, ToSchema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentCustomer {
    /// `context.groupId`, or `groupId`: the account, as B2B billing counts it.
    #[default]
    Group,
    /// `userId`.
    User,
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct SegmentQuery {
    /// Which identity is the customer's alias: `group` (the default) or `user`.
    #[param(inline)]
    pub customer: Option<SegmentCustomer>,
    /// Accept timestamps up to 13 months ago instead of 24 hours.
    #[serde(default)]
    pub allow_backfilling: bool,
}

/// A Segment `track` event; any other field Segment sends is ignored.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SegmentTrackEvent {
    /// Segment's id of the message: the event's `event_id`, so Segment's retries count once.
    pub message_id: String,
    /// `track`, when sent.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// The event's name, as written in Segment: the event code metrics read.
    pub event: String,
    /// The user: the customer's alias with `customer=user`.
    #[serde(default)]
    pub user_id: Option<String>,
    /// A group id mapped to the top level, read before `context.groupId`.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Segment's context: only its `groupId` is read.
    #[serde(default)]
    pub context: Option<SegmentContext>,
    /// RFC 3339; the time it is received when left out.
    #[serde(default)]
    pub timestamp: Option<String>,
    /// Nested objects are flattened to dotted keys (`plan.tier`), arrays kept as JSON text, nulls
    /// dropped.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub properties: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SegmentContext {
    /// The group (account) the event is for: the customer's alias by default.
    #[serde(default)]
    pub group_id: Option<String>,
}

fn flatten(prefix: String, value: Value, out: &mut HashMap<String, String>) {
    match value {
        Value::Null => {}
        Value::Object(map) => {
            for (key, value) in map {
                let key = if prefix.is_empty() {
                    key
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(key, value, out);
            }
        }
        Value::String(text) => {
            out.insert(prefix, text);
        }
        Value::Number(number) => {
            out.insert(prefix, number.to_string());
        }
        Value::Bool(flag) => {
            out.insert(prefix, flag.to_string());
        }
        array @ Value::Array(_) => {
            out.insert(prefix, array.to_string());
        }
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn segment_event(
    raw: Value,
    index: usize,
    customer: SegmentCustomer,
) -> Result<grpc::Event, IngestRejection> {
    let event_id = raw
        .get("messageId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let refused = |field: &str, message: String| IngestRejection {
        index,
        path: format!("/{index}{field}"),
        event_id: event_id.clone(),
        message,
    };
    let event: SegmentTrackEvent =
        serde_json::from_value(raw).map_err(|error| refused("", error.to_string()))?;
    let id_chars = event.message_id.chars().count();
    if id_chars == 0 || id_chars > MAX_MESSAGE_ID_CHARS {
        return Err(refused(
            "/messageId",
            format!("A messageId is 1 to {MAX_MESSAGE_ID_CHARS} characters"),
        ));
    }
    if let Some(kind) = event.kind.as_deref().filter(|kind| *kind != "track") {
        return Err(refused(
            "/type",
            format!("Only track events are usage; this one is `{kind}`"),
        ));
    }
    validate_code(&event.event).map_err(|error| refused("/event", error.to_string()))?;
    let alias = match customer {
        SegmentCustomer::Group => non_blank(event.group_id)
            .or_else(|| non_blank(event.context.and_then(|context| context.group_id)))
            .ok_or_else(|| {
                refused(
                    "/context/groupId",
                    "The event has no groupId to name its customer by (or set `customer=user`)"
                        .to_owned(),
                )
            })?,
        SegmentCustomer::User => non_blank(event.user_id).ok_or_else(|| {
            refused(
                "/userId",
                "The event has no userId to name its customer by".to_owned(),
            )
        })?,
    };
    let mut properties = HashMap::new();
    flatten(
        String::new(),
        Value::Object(event.properties.unwrap_or_default()),
        &mut properties,
    );
    Ok(grpc::Event {
        id: event.message_id,
        code: event.event,
        customer_id: Some(grpc::event::CustomerId::ExternalCustomerAlias(alias)),
        timestamp: event.timestamp.unwrap_or_default(),
        properties,
    })
}

/// Ingest Segment events
///
/// Ingests Segment `track` events as usage, for Segment's Webhooks (Actions) destination: one
/// event, or a batch of up to 4,000 (the body 1 MB at most). Map only the fields below to stay
/// under it. Each event's `messageId` is its `event_id`, its `event` name its code, and its
/// customer the alias in `context.groupId` (or `groupId`), or in `userId` with `customer=user`.
/// `properties` are flattened (`{"plan": {"tier": "pro"}}` is `plan.tier`). The `timestamp`
/// rules are `/events/ingest`'s.
///
/// The events accepted are ingested, and the `200` names the ones refused in `rejected`, at
/// their pointer in the body (`/3/userId`). Sending a batch again is safe: events are
/// deduplicated by `messageId` and customer.
#[utoipa::path(
    post,
    tag = "Events",
    path = "/api/v2/events/segment",
    params(SegmentQuery),
    request_body(content = Vec<SegmentTrackEvent>, example = json!(crate::v2::examples::request("ingest_segment_events"))),
    responses(
        (status = 200, description = "The events accepted, and those rejected", body = IngestEventsResult, example = json!(crate::v2::examples::response("ingest_segment_events"))),
        (status = 422, description = "The batch is empty or too large, or not JSON events", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests or events, retry later", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "Metering is unavailable, retry later", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn ingest_segment_events(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<SegmentQuery>,
    Body(body): Body<Value>,
) -> Result<Json<IngestEventsResult>, Problem> {
    let raw = match body {
        Value::Array(events) => events,
        event @ Value::Object(_) => vec![event],
        _ => {
            return Err(Problem::field(
                "",
                "invalid",
                "The body is a Segment event or an array of them",
            ));
        }
    };
    let total = raw.len();
    if total == 0 || total > MAX_SEGMENT_EVENTS {
        return Err(Problem::field(
            "",
            "invalid",
            format!("A batch has 1 to {MAX_SEGMENT_EVENTS} events; this one has {total}"),
        ));
    }
    let customer = query.customer.unwrap_or_default();
    let mut rejected = Vec::new();
    let mut forwarded = Vec::with_capacity(total);
    for (index, event) in raw.into_iter().enumerate() {
        match segment_event(event, index, customer) {
            Ok(event) => forwarded.push((index, event)),
            Err(rejection) => rejected.push(rejection),
        }
    }
    for chunk in forwarded.chunks(MAX_EVENTS) {
        let mut positions: HashMap<&str, VecDeque<usize>> = HashMap::new();
        for (index, event) in chunk {
            positions.entry(&event.id).or_default().push_back(*index);
        }
        let result = crate::events::router::ingest(
            &state,
            &auth,
            chunk.iter().map(|(_, event)| event.clone()).collect(),
            query.allow_backfilling,
            false,
        )
        .await
        .map_err(Problem::from)?;
        rejected.extend(result.failures.into_iter().map(|failure| {
            let index = positions
                .get_mut(failure.event_id.as_str())
                .and_then(VecDeque::pop_front);
            IngestRejection {
                index: index.unwrap_or(total),
                path: index.map(|index| format!("/{index}")).unwrap_or_default(),
                event_id: failure.event_id,
                message: failure.reason,
            }
        }));
    }
    rejected.sort_by_key(|rejection| rejection.index);
    Ok(Json(IngestEventsResult {
        accepted: total - rejected.len(),
        rejected,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn track(extra: Value) -> Value {
        let mut event = json!({
            "messageId": "ajs-1",
            "type": "track",
            "event": "Completion",
            "userId": "u-42",
            "context": {"groupId": "northwind", "library": {"name": "analytics.js"}},
            "timestamp": "2026-11-26T09:12:44.249Z",
            "properties": {"tokens": 1832, "model": {"name": "large", "beta": true}, "tags": ["a"], "gone": null},
            "anonymousId": "anon"
        });
        if let (Value::Object(event), Value::Object(extra)) = (&mut event, extra) {
            event.extend(extra);
        }
        event
    }

    #[test]
    fn a_track_event_is_ingested_under_its_group_by_default() {
        let event = segment_event(track(json!({})), 0, SegmentCustomer::Group).unwrap();
        assert_eq!(event.id, "ajs-1");
        assert_eq!(event.code, "Completion");
        assert_eq!(
            event.customer_id,
            Some(grpc::event::CustomerId::ExternalCustomerAlias(
                "northwind".to_owned()
            ))
        );
        assert_eq!(event.timestamp, "2026-11-26T09:12:44.249Z");
        assert_eq!(event.properties["tokens"], "1832");
        assert_eq!(event.properties["model.name"], "large");
        assert_eq!(event.properties["model.beta"], "true");
        assert_eq!(event.properties["tags"], r#"["a"]"#);
        assert!(!event.properties.contains_key("gone"));
    }

    #[test]
    fn a_top_level_group_id_comes_before_the_context() {
        let event =
            segment_event(track(json!({"groupId": "acme"})), 0, SegmentCustomer::Group).unwrap();
        assert_eq!(
            event.customer_id,
            Some(grpc::event::CustomerId::ExternalCustomerAlias(
                "acme".to_owned()
            ))
        );
    }

    #[test]
    fn the_user_names_the_customer_when_asked() {
        let event = segment_event(track(json!({})), 0, SegmentCustomer::User).unwrap();
        assert_eq!(
            event.customer_id,
            Some(grpc::event::CustomerId::ExternalCustomerAlias(
                "u-42".to_owned()
            ))
        );
    }

    #[test]
    fn an_event_is_refused_at_its_field() {
        let no_group =
            segment_event(track(json!({"context": {}})), 3, SegmentCustomer::Group).unwrap_err();
        assert_eq!(no_group.path, "/3/context/groupId");
        assert_eq!(no_group.event_id, "ajs-1");

        let no_user =
            segment_event(track(json!({"userId": null})), 1, SegmentCustomer::User).unwrap_err();
        assert_eq!(no_user.path, "/1/userId");

        let identify = segment_event(
            track(json!({"type": "identify"})),
            0,
            SegmentCustomer::Group,
        )
        .unwrap_err();
        assert_eq!(identify.path, "/0/type");

        let unnamed =
            segment_event(track(json!({"messageId": ""})), 2, SegmentCustomer::Group).unwrap_err();
        assert_eq!(unnamed.path, "/2/messageId");

        let malformed = segment_event(
            json!({"messageId": "m", "event": 4}),
            5,
            SegmentCustomer::Group,
        )
        .unwrap_err();
        assert_eq!(malformed.path, "/5");
        assert_eq!(malformed.event_id, "m");
    }

    #[test]
    fn a_mapped_event_without_type_or_properties_is_kept() {
        let event = segment_event(
            json!({"messageId": "m1", "event": "api_call", "userId": "u1"}),
            0,
            SegmentCustomer::User,
        )
        .unwrap();
        assert!(event.properties.is_empty());
        assert_eq!(event.timestamp, "");
    }
}
