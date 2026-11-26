//! `/api/v2/webhooks` (ADR-0013 §15): the endpoints Meteroid POSTs signed event payloads to,
//! created, read, patched and archived, their signing secret read and rotated, and their
//! deliveries listed newest first by keyset in SQL, read one by one and sent again, each on the
//! store call v1 uses.
//!
//! An endpoint created here records the request's `Meteroid-Version` as its `api_version` (the
//! latest version when the request names none, as a new integration wants), which pins its
//! payloads' shape: from `2026-12-15` (`V2_WEBHOOK_PAYLOADS`) v2's envelopes
//! (`webhook_payloads`), before it v1's payloads. Their `type` is the event type it subscribes to
//! (`invoice.paid`) in either shape.
use crate::AppState;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::holds_secret;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset, page, refused_cursor};
use crate::v2::problem::Problem;
use crate::v2::version::{ApiVersion, VERSION_HEADER, VERSIONS, latest_version};
use crate::webhooks::out_model::WebhookOutEventTypeEnum;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use common_domain::ids::{BaseId, EventId, TenantId, WebhookDeliveryId, WebhookEndpointId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::services::webhooks_out::{
    check_endpoint_input, check_headers, check_rate_limit, tenant_max_in_flight,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::enums::{
    WebhookDeliveryStatus as DomainDeliveryStatus,
    WebhookEndpointDisabledReason as DomainDisabledReason,
};
use meteroid_store::domain::webhooks_out::{
    WebhookDelivery as DomainDelivery, WebhookEndpoint as DomainEndpoint, WebhookEndpointNew,
    WebhookEndpointPatch, WebhookHeaderInput as DomainHeaderInput,
};
use meteroid_store::repositories::WebhookOutInterface;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// How a consumer verifies a delivery, published after the Webhooks tag's own text: the
/// Standard Webhooks scheme `signing::build_headers` signs with.
pub const SIGNATURE_VERIFICATION: &str = "**Verifying a delivery.** Every delivery is signed \
    as the Standard Webhooks specification describes (https://www.standardwebhooks.com), so its \
    libraries verify it \
    (https://github.com/standard-webhooks/standard-webhooks/tree/main/libraries): pass them \
    the endpoint's secret as it reads, the headers and the raw body. Each delivery \
    carries three headers. `webhook-id` is the event's id (`evt_…`, its payload's `id`), the \
    same on every attempt and resend of it: deduplicate by it. `webhook-timestamp` is the Unix \
    time, in seconds, the attempt was sent. `webhook-signature` is `v1,` followed by the base64 \
    (standard alphabet, padded) of the HMAC-SHA256 of `<webhook-id>.<webhook-timestamp>.<body>`, \
    the body's bytes exactly as received: verify them before parsing. The HMAC key is the \
    secret's bytes, the base64 after its `whsec_` prefix decoded, not the string itself. While \
    a rotated-out secret still signs (24 hours after its rotation), the header holds one \
    signature per secret, separated by spaces, the current one first: accept the delivery when \
    any of them matches, compared in constant time. Refuse a timestamp more than 5 minutes away \
    from your clock, so a delivery captured on the way cannot be replayed later. An endpoint \
    migrated from Svix also receives the same three values as `svix-id`, `svix-timestamp` and \
    `svix-signature`.";

// ── Model ───────────────────────────────────────────────────────────

/// Why an endpoint receives nothing.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisabledReason {
    /// Disabled by a patch or from the dashboard.
    Manual,
    /// Failing for too long: re-enable it once it answers again.
    AutoFailures,
    /// It answered 410 Gone.
    Gone,
}

impl From<DomainDisabledReason> for DisabledReason {
    fn from(reason: DomainDisabledReason) -> Self {
        match reason {
            DomainDisabledReason::Manual => DisabledReason::Manual,
            DomainDisabledReason::AutoFailures => DisabledReason::AutoFailures,
            DomainDisabledReason::Gone => DisabledReason::Gone,
        }
    }
}

impl From<DisabledReason> for DomainDisabledReason {
    fn from(reason: DisabledReason) -> Self {
        match reason {
            DisabledReason::Manual => DomainDisabledReason::Manual,
            DisabledReason::AutoFailures => DomainDisabledReason::AutoFailures,
            DisabledReason::Gone => DomainDisabledReason::Gone,
        }
    }
}

/// Where a delivery is in its retry cycle.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// Waiting for its next attempt.
    Pending,
    /// Being sent.
    InFlight,
    /// The endpoint answered 2xx.
    Succeeded,
    /// Every attempt failed.
    Failed,
    /// The endpoint was archived or disabled before it was sent.
    Cancelled,
}

impl From<DomainDeliveryStatus> for DeliveryStatus {
    fn from(status: DomainDeliveryStatus) -> Self {
        match status {
            DomainDeliveryStatus::Pending => DeliveryStatus::Pending,
            DomainDeliveryStatus::InFlight => DeliveryStatus::InFlight,
            DomainDeliveryStatus::Succeeded => DeliveryStatus::Succeeded,
            DomainDeliveryStatus::Failed => DeliveryStatus::Failed,
            DomainDeliveryStatus::Cancelled => DeliveryStatus::Cancelled,
        }
    }
}

impl From<DeliveryStatus> for DomainDeliveryStatus {
    fn from(status: DeliveryStatus) -> Self {
        match status {
            DeliveryStatus::Pending => DomainDeliveryStatus::Pending,
            DeliveryStatus::InFlight => DomainDeliveryStatus::InFlight,
            DeliveryStatus::Succeeded => DomainDeliveryStatus::Succeeded,
            DeliveryStatus::Failed => DomainDeliveryStatus::Failed,
            DeliveryStatus::Cancelled => DomainDeliveryStatus::Cancelled,
        }
    }
}

/// A header sent with every delivery. A sensitive one never reads its value back: `set` says
/// whether it has one.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct WebhookHeader {
    /// The header's name, as sent.
    pub name: String,
    /// Its value; absent on a sensitive header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Write-only: its value is stored encrypted and never read back.
    pub sensitive: bool,
    /// Whether it has a value. A sensitive header without one keeps the endpoint from being
    /// enabled (`needs_setup`).
    pub set: bool,
}

/// Where Meteroid POSTs the signed payloads of the events it subscribes to.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct WebhookEndpoint {
    /// The endpoint's id, `whe_…`.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: WebhookEndpointId,
    /// The HTTPS URL deliveries are POSTed to.
    pub url: String,
    /// What the endpoint is for, as the tenant wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The event types it receives, as a payload's `type` names them (`invoice.paid`). Empty:
    /// every event type but the beta ones, sent only to endpoints naming them.
    #[schema(value_type = Vec<WebhookOutEventTypeEnum>)]
    pub event_types: Vec<String>,
    /// A disabled endpoint receives nothing: its deliveries are cancelled.
    pub disabled: bool,
    /// Why it is disabled; absent while it is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<DisabledReason>,
    /// Failures since the last success; 0 again on any 2xx.
    pub consecutive_failures: i32,
    /// Deliveries in flight at once, at most; set from the tenant's environment.
    pub max_in_flight: i32,
    /// The headers sent with every delivery, beside the signature's.
    pub headers: Vec<WebhookHeader>,
    /// A sensitive header waits for its value: the endpoint cannot be enabled before.
    pub needs_setup: bool,
    /// Deliveries started per second, at most.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_sec: Option<i32>,
    /// Unreachable several times in a row: nothing is sent before then, then one delivery at a
    /// time until it answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<ApiDateTime>,
    /// When it last answered a delivery with a 2xx.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<ApiDateTime>,
    /// When a delivery to it last failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<ApiDateTime>,
    /// The API version its payloads are rendered at: the `Meteroid-Version` it was created at,
    /// or the one a patch set. From `2026-12-15` on, v2's envelopes, whose `data` reads as the
    /// resource's v2 GET; before it, v1's payloads. Absent on an endpoint created through v1 (or
    /// the dashboard before it pinned versions), which keeps v1's payloads. A pinned endpoint
    /// never changes shape: only a patch moves it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-12-15")]
    pub api_version: Option<String>,
    /// When it was created.
    pub created_at: ApiDateTime,
    /// When it last changed; absent if it never did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<ApiDateTime>,
    /// Set by `POST …/archive`, read only in its answer: an archived endpoint is no longer read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<ApiDateTime>,
}

crate::v2_list!(WebhookEndpointList, WebhookEndpoint);

/// The endpoint created, with its signing secret: read it again with `GET …/secret`.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CreatedWebhookEndpoint {
    #[serde(flatten)]
    pub endpoint: WebhookEndpoint,
    /// The signing secret, `whsec_` and the base64 of 24 random bytes, as `WebhookSecret`'s.
    pub secret: String,
}

/// The secret deliveries are signed with.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct WebhookSecret {
    /// `whsec_` followed by the base64 (standard alphabet) of 24 random bytes. The HMAC-SHA256
    /// key is those bytes, the part after `whsec_` decoded; a Standard Webhooks library takes
    /// the whole string. A delivery's `webhook-signature` is `v1,` and the base64 of the HMAC of
    /// `<webhook-id>.<webhook-timestamp>.<body>`; after a rotation it holds one signature per
    /// secret still signing, separated by spaces (the Webhooks tag says how to verify one).
    /// Keep it secret: anyone holding it can sign a delivery.
    #[schema(example = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw")]
    pub secret: String,
}

/// One event queued for one endpoint.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct WebhookDelivery {
    /// The delivery's id, `whd_…`: the one `POST /webhooks/deliveries/{id}/resend` takes.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: WebhookDeliveryId,
    /// The event delivered: its payload's `id`.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub event: EventId,
    /// The endpoint it is sent to.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub endpoint: WebhookEndpointId,
    /// Its payload's `type`.
    pub event_type: String,
    /// Where it is in its retry cycle.
    pub status: DeliveryStatus,
    /// Attempts made so far, the resends' included.
    pub attempts: i32,
    /// When the next attempt is due; absent once the delivery ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<ApiDateTime>,
    /// The HTTP status of the last answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_response_status: Option<i32>,
    /// Why the last attempt failed, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Made by a resend or a test event.
    pub manual: bool,
    /// When it was queued.
    pub created_at: ApiDateTime,
    /// When it ended: succeeded, failed or cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<ApiDateTime>,
}

crate::v2_list!(WebhookDeliveryList, WebhookDelivery);

/// A header to send with every delivery. A sensitive one is write-only; in a patch, one sent
/// without a value keeps the value it has.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct WebhookHeaderInput {
    /// The header's name, unique ignoring case. Meteroid sets `webhook-*`, `svix-*`,
    /// `proxy-*`, `content-type`, `user-agent` and the connection's headers (`host`,
    /// `content-length`…): those are refused.
    pub name: String,
    /// Its value, without a line break. Required, but on a sensitive header of a patch, where
    /// leaving it out keeps the value it has.
    #[serde(default)]
    pub value: Option<String>,
    /// Write-only: its value is stored encrypted and never read back.
    #[serde(default)]
    pub sensitive: bool,
}

/// An endpoint to create: where to POST, which event types, and the headers to send.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateWebhookEndpoint {
    /// HTTPS; private and loopback addresses are refused unless the instance allows them.
    pub url: String,
    /// What the endpoint is for; blank reads as none.
    #[serde(default)]
    pub description: Option<String>,
    /// As a payload's `type` names them; every event type but the beta ones when empty.
    #[serde(default)]
    #[schema(value_type = Vec<WebhookOutEventTypeEnum>)]
    pub event_types: Vec<String>,
    /// Headers to send with every delivery.
    #[serde(default)]
    pub headers: Vec<WebhookHeaderInput>,
    /// Deliveries started per second, at most: 1 to 1000; none by default.
    #[serde(default)]
    pub rate_limit_per_sec: Option<i32>,
}

/// A merge patch: a field left out is unchanged, `null` clears it; `event_types` and `headers`
/// replace the lists. Enabling an endpoint again resets its failure count.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateWebhookEndpoint {
    /// Another HTTPS URL, checked as on create.
    #[serde(default)]
    pub url: Option<String>,
    /// `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Replaces the list; empty receives every event type but the beta ones.
    #[serde(default)]
    #[schema(value_type = Option<Vec<WebhookOutEventTypeEnum>>)]
    pub event_types: Option<Vec<String>>,
    /// `true` disables it (`disabled_reason: manual`); `false` enables it again and resets its
    /// failure count.
    #[serde(default)]
    pub disabled: Option<bool>,
    /// Replaces the list; a sensitive header sent without a value keeps the one it has.
    #[serde(default)]
    pub headers: Option<Vec<WebhookHeaderInput>>,
    /// 1 to 1000; `null` removes the limit.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<i32>)]
    pub rate_limit_per_sec: Option<Option<i32>>,
    /// Pins its payloads to another published version (`2026-12-15` for v2's), from its next
    /// event on; it cannot be removed.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub api_version: Option<Option<String>>,
}

#[derive(Deserialize, IntoParams, Debug, Clone)]
#[into_params(parameter_in = Query)]
pub struct DeliveryListQuery {
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page. On this list a cursor reads only as long as its
    /// delivery is kept: deliveries are purged with their event 30 days after it by default (a
    /// self-hosted instance sets `WEBHOOK_RETENTION_DAYS`, 2 at least), and past that the cursor
    /// is refused as one of another list is.
    pub after: Option<String>,
    /// Only the deliveries in one of these states: repeat the parameter for several
    /// (`?status=pending&status=failed`).
    #[param(style = Form, explode)]
    pub status: Option<Vec<DeliveryStatus>>,
}

// ── Mapping ─────────────────────────────────────────────────────────

fn endpoint(endpoint: DomainEndpoint) -> WebhookEndpoint {
    let needs_setup = endpoint.needs_setup();
    WebhookEndpoint {
        id: endpoint.id,
        url: endpoint.url,
        description: endpoint.description,
        event_types: endpoint.event_types,
        disabled: endpoint.disabled,
        disabled_reason: endpoint.disabled_reason.map(Into::into),
        consecutive_failures: endpoint.consecutive_failures,
        max_in_flight: endpoint.max_in_flight,
        headers: endpoint
            .headers
            .into_iter()
            .map(|header| WebhookHeader {
                name: header.name,
                value: header.value,
                sensitive: header.sensitive,
                set: header.set,
            })
            .collect(),
        needs_setup,
        rate_limit_per_sec: endpoint.rate_limit_per_sec,
        paused_until: endpoint.paused_until.map(Into::into),
        last_success_at: endpoint.last_success_at.map(Into::into),
        last_failure_at: endpoint.last_failure_at.map(Into::into),
        api_version: endpoint.api_version,
        created_at: endpoint.created_at.into(),
        updated_at: endpoint.updated_at.map(Into::into),
        archived_at: endpoint.archived_at.map(Into::into),
    }
}

fn delivery(delivery: DomainDelivery, event_type: String) -> WebhookDelivery {
    WebhookDelivery {
        id: delivery.id,
        event: delivery.message_id,
        endpoint: delivery.endpoint_id,
        event_type,
        status: delivery.status.into(),
        attempts: delivery.attempt_count,
        next_attempt_at: delivery.next_attempt_at.map(Into::into),
        last_response_status: delivery.last_response_status.map(i32::from),
        last_error: delivery.last_error,
        manual: delivery.manual,
        created_at: delivery.created_at.into(),
        completed_at: delivery.completed_at.map(Into::into),
    }
}

fn header_in(header: WebhookHeaderInput) -> DomainHeaderInput {
    DomainHeaderInput {
        name: header.name.trim().to_owned(),
        value: header.value,
        sensitive: header.sensitive,
    }
}

// ── Checks ──────────────────────────────────────────────────────────

/// The event types asked, each one an endpoint can receive: written as a payload's `type` names
/// it, which is how they are stored and matched.
fn event_types(asked: &[String], path: &str) -> Result<Vec<String>, Problem> {
    check_endpoint_input(None, None, Some(asked))
        .map_err(|message| Problem::field(path, "invalid", message))?;
    asked
        .iter()
        .enumerate()
        .map(|(index, name)| {
            WebhookOutEventTypeEnum::from_str(name.trim())
                .ok()
                .filter(|event| {
                    !crate::v2::document::RETIRED_EVENT_TYPES.contains(&event.to_string().as_str())
                })
                .map(|event| event.to_string())
                .ok_or_else(|| {
                    Problem::field(
                        format!("{path}/{index}"),
                        "invalid",
                        format!("'{}' is not an event type", name.trim()),
                    )
                })
        })
        .collect()
}

/// A destination the delivery worker would send to: the same SSRF guards, refused at `/url`.
fn url(state: &AppState, url: &str) -> Result<String, Problem> {
    let url = url.trim();
    check_endpoint_input(Some(url), None, None)
        .map_err(|message| Problem::field("/url", "invalid", message))?;
    crate::webhooks::endpoints::router::validate_destination(state, url).map_err(|error| {
        match error {
            RestApiError::InvalidInput(message) => Problem::field("/url", "invalid", message),
            other => Problem::from(other),
        }
    })?;
    Ok(url.to_owned())
}

/// A description, blank read as none.
fn description(asked: Option<String>) -> Result<Option<String>, Problem> {
    let asked = asked.filter(|text| !text.trim().is_empty());
    check_endpoint_input(None, asked.as_deref(), None)
        .map_err(|message| Problem::field("/description", "invalid", message))?;
    Ok(asked)
}

fn headers(
    asked: Vec<WebhookHeaderInput>,
    creating: bool,
) -> Result<Vec<DomainHeaderInput>, Problem> {
    let headers: Vec<DomainHeaderInput> = asked.into_iter().map(header_in).collect();
    check_headers(&headers, creating)
        .map_err(|message| Problem::field("/headers", "invalid", message))?;
    Ok(headers)
}

fn rate_limit(asked: Option<i32>) -> Result<Option<i32>, Problem> {
    check_rate_limit(asked)
        .map_err(|message| Problem::field("/rate_limit_per_sec", "invalid", message))?;
    Ok(asked)
}

/// A published version an endpoint's payloads are pinned to, as `Meteroid-Version` names it.
fn api_version(asked: Option<String>) -> Result<String, Problem> {
    let asked = asked.ok_or_else(|| {
        Problem::field(
            "/api_version",
            "invalid",
            "An endpoint's API version can change, not be removed",
        )
    })?;
    let asked = asked.trim();
    VERSIONS
        .iter()
        .find(|version| **version == asked)
        .map(|version| (*version).to_owned())
        .ok_or_else(|| {
            Problem::field(
                "/api_version",
                "invalid",
                format!(
                    "'{asked}' is not a published API version: {}",
                    VERSIONS.join(", ")
                ),
            )
        })
}

fn endpoint_id(raw: &str) -> Result<WebhookEndpointId, Problem> {
    ids::plain_id(raw, "webhook endpoint")
}

async fn find_endpoint(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<DomainEndpoint, Problem> {
    let id = endpoint_id(raw)?;
    state
        .store
        .get_webhook_endpoint(tenant_id, id)
        .await
        .map_err(|error| match Problem::from(error) {
            problem if problem.status == 404 => {
                Problem::not_found(format!("webhook endpoint {}", raw.trim()))
            }
            problem => problem,
        })
}

// ── Handlers ────────────────────────────────────────────────────────

/// List webhook endpoints
///
/// The tenant's endpoints in creation order, paged by cursor (a tenant has 50 at most).
#[utoipa::path(
    get,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints",
    params(PageQuery),
    responses(
        (status = 200, description = "A page of webhook endpoints", body = WebhookEndpointList, example = json!(crate::v2::examples::response("list_webhook_endpoints"))),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_webhook_endpoints(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<WebhookEndpointList>, Problem> {
    let endpoints = state.store.list_webhook_endpoints(auth.tenant_id).await?;
    Ok(Json(
        page(endpoints, |item| item.id.as_uuid(), &query)?
            .map(endpoint)
            .into(),
    ))
}

/// Create a webhook endpoint
///
/// Answers the endpoint with its signing secret. A retry under the same `Idempotency-Key` does
/// not create another, and does not answer the secret again: read it with `GET …/secret`. Its
/// `api_version` is the request's `Meteroid-Version`, or the latest version when the request
/// sends none: its payloads keep that shape until it is changed.
#[utoipa::path(
    post,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints",
    request_body(content = CreateWebhookEndpoint, example = json!(crate::v2::examples::request("create_webhook_endpoint"))),
    responses(
        (status = 201, description = "Created, with its signing secret", body = CreatedWebhookEndpoint, example = json!(crate::v2::examples::response("create_webhook_endpoint"))),
        (status = 409, description = "The tenant has the most endpoints it may have", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused: a URL the worker would not send to, an unknown event type…", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_webhook_endpoint(
    Extension(auth): Extension<AuthorizedAsTenant>,
    version: Option<Extension<ApiVersion>>,
    request_headers: http::HeaderMap,
    State(state): State<AppState>,
    Body(body): Body<CreateWebhookEndpoint>,
) -> Result<Response, Problem> {
    // A version asked is pinned; none asked pins the latest, never the first.
    let version = match version {
        Some(Extension(version)) if request_headers.contains_key(VERSION_HEADER) => version,
        _ => latest_version(),
    };
    let url = url(&state, &body.url)?;
    let description = description(body.description)?;
    let event_types = event_types(&body.event_types, "/event_types")?;
    let headers = headers(body.headers, true)?;
    let rate_limit_per_sec = rate_limit(body.rate_limit_per_sec)?;
    let max_in_flight = tenant_max_in_flight(&state.store, auth.tenant_id).await?;

    let (created, secret) = state
        .store
        .create_webhook_endpoint(
            auth.tenant_id,
            WebhookEndpointNew {
                url,
                description,
                event_types,
                max_in_flight,
                headers,
                rate_limit_per_sec,
                api_version: Some(version.as_header()),
            },
        )
        .await?;
    Ok(holds_secret(
        (
            StatusCode::CREATED,
            Json(CreatedWebhookEndpoint {
                endpoint: endpoint(created),
                secret: secret.expose_secret().to_string(),
            }),
        )
            .into_response(),
    ))
}

/// Get a webhook endpoint
///
/// An archived endpoint is not found. Its signing secret is read with `GET …/secret`.
#[utoipa::path(
    get,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints/{id}",
    params(("id" = String, Path, description = "Webhook endpoint id")),
    responses(
        (status = 200, description = "The webhook endpoint", body = WebhookEndpoint, example = json!(crate::v2::examples::response("get_webhook_endpoint"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_webhook_endpoint(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WebhookEndpoint>, Problem> {
    Ok(Json(endpoint(
        find_endpoint(&state, auth.tenant_id, &id).await?,
    )))
}

/// Update a webhook endpoint
///
/// A JSON merge patch. Enabling an endpoint again (`disabled: false`) resets its failure count;
/// one whose sensitive header has no value cannot be enabled. `api_version` pins its payloads to
/// another published version: the events after the patch are rendered at it, those already
/// queued keep the shape they were rendered in.
#[utoipa::path(
    patch,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints/{id}",
    params(("id" = String, Path, description = "Webhook endpoint id")),
    request_body(content = UpdateWebhookEndpoint, content_type = "application/merge-patch+json", example = json!(crate::v2::examples::request("update_webhook_endpoint"))),
    responses(
        (status = 200, description = "Updated", body = WebhookEndpoint, example = json!(crate::v2::examples::response("update_webhook_endpoint"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_webhook_endpoint(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<UpdateWebhookEndpoint>,
) -> Result<Json<WebhookEndpoint>, Problem> {
    let found = find_endpoint(&state, auth.tenant_id, &id).await?;
    let url = body
        .url
        .as_deref()
        .map(|asked| url(&state, asked))
        .transpose()?;
    let description = body.description.map(description).transpose()?;
    let event_types = body
        .event_types
        .as_deref()
        .map(|asked| event_types(asked, "/event_types"))
        .transpose()?;
    let headers = body
        .headers
        .map(|asked| headers(asked, false))
        .transpose()?;
    let rate_limit_per_sec = body.rate_limit_per_sec.map(rate_limit).transpose()?;
    let api_version = body.api_version.map(api_version).transpose()?;

    let updated = state
        .store
        .update_webhook_endpoint(
            auth.tenant_id,
            WebhookEndpointPatch {
                id: found.id,
                url,
                description,
                event_types,
                disabled: body.disabled,
                headers,
                rate_limit_per_sec,
                api_version,
            },
        )
        .await?;
    Ok(Json(endpoint(updated)))
}

/// Archive a webhook endpoint
///
/// It receives nothing from then on: its pending deliveries are cancelled, and it is no longer
/// read, listed or counted against the tenant's 50. Answers the endpoint archived; archiving is
/// final.
#[utoipa::path(
    post,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints/{id}/archive",
    params(("id" = String, Path, description = "Webhook endpoint id")),
    responses(
        (status = 200, description = "Archived", body = WebhookEndpoint, example = json!(crate::v2::examples::response("archive_webhook_endpoint"))),
        (status = 404, description = "Not found, or archived already", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_webhook_endpoint(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WebhookEndpoint>, Problem> {
    let found = find_endpoint(&state, auth.tenant_id, &id).await?;
    let archived = state
        .store
        .archive_webhook_endpoint(auth.tenant_id, found.id)
        .await?;
    Ok(Json(endpoint(archived)))
}

/// Get a webhook endpoint's signing secret
///
/// The current secret, the one a delivery's first signature is made with. A secret rotated out
/// within the last 24 hours still signs beside it but is not read back.
#[utoipa::path(
    get,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints/{id}/secret",
    params(("id" = String, Path, description = "Webhook endpoint id")),
    responses(
        (status = 200, description = "The signing secret", body = WebhookSecret, example = json!(crate::v2::examples::response("get_webhook_endpoint_secret"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_webhook_endpoint_secret(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WebhookSecret>, Problem> {
    let found = find_endpoint(&state, auth.tenant_id, &id).await?;
    let secret = state
        .store
        .get_webhook_endpoint_secret(auth.tenant_id, found.id)
        .await?;
    Ok(Json(WebhookSecret {
        secret: secret.expose_secret().to_string(),
    }))
}

/// Rotate a webhook endpoint's signing secret
///
/// The secret it replaces signs beside the new one for 24 hours, as does every secret rotated
/// out within those 24 hours, so a consumer rolls over without dropping events however often it
/// rotates (10 such secrets at most: past that, rotate again once the oldest expires). A retry
/// under the same `Idempotency-Key` rotates once and does not answer the secret again: read it
/// with `GET …/secret`.
#[utoipa::path(
    post,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints/{id}/rotate-secret",
    params(("id" = String, Path, description = "Webhook endpoint id")),
    responses(
        (status = 200, description = "The new signing secret", body = WebhookSecret, example = json!(crate::v2::examples::response("rotate_webhook_endpoint_secret"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Rotated 10 times in 24 hours, each secret still signing", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn rotate_webhook_endpoint_secret(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Problem> {
    let found = find_endpoint(&state, auth.tenant_id, &id).await?;
    let secret = state
        .store
        .rotate_webhook_endpoint_secret(auth.tenant_id, found.id)
        .await?;
    Ok(holds_secret(
        Json(WebhookSecret {
            secret: secret.expose_secret().to_string(),
        })
        .into_response(),
    ))
}

/// List a webhook endpoint's deliveries
///
/// Newest first, paged by cursor; narrowed to some `status`es.
#[utoipa::path(
    get,
    tag = "Webhooks",
    path = "/api/v2/webhooks/endpoints/{id}/deliveries",
    params(
        ("id" = String, Path, description = "Webhook endpoint id"),
        DeliveryListQuery,
    ),
    responses(
        (status = 200, description = "A page of deliveries, newest first", body = WebhookDeliveryList, example = json!(crate::v2::examples::response("list_webhook_deliveries"))),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_webhook_deliveries(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeliveryListQuery>,
) -> Result<Json<WebhookDeliveryList>, Problem> {
    let paging = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = paging.limit()?;
    let before = paging
        .cursor()?
        .map(|Cursor(before)| WebhookDeliveryId::from(before));
    let found = find_endpoint(&state, auth.tenant_id, &id).await?;
    let rows = state
        .store
        .list_webhook_deliveries_before(
            auth.tenant_id,
            found.id,
            query
                .status
                .map(|statuses| statuses.into_iter().map(Into::into).collect()),
            before,
            limit + 1,
        )
        .await?
        // A delivery purged since (or never the endpoint's): no page from it would be exact.
        .ok_or_else(refused_cursor)?;
    Ok(Json(
        keyset(rows, limit, |item| item.delivery.id.as_uuid())
            .map(|item| delivery(item.delivery, item.event_type))
            .into(),
    ))
}

/// The tenant's delivery of this id, with its event's type; another tenant's is not found.
async fn find_delivery(
    state: &AppState,
    tenant_id: TenantId,
    id: &str,
) -> Result<meteroid_store::domain::webhooks_out::WebhookDeliveryDetails, Problem> {
    let id: WebhookDeliveryId = ids::plain_id(id, "webhook delivery")?;
    state
        .store
        .get_webhook_delivery(tenant_id, id)
        .await
        .map_err(|error| match Problem::from(error) {
            problem if problem.status == 404 => {
                Problem::not_found(format!("webhook delivery {id}"))
            }
            problem => problem,
        })
}

/// Get a webhook delivery
///
/// One event queued for one endpoint, as its endpoint's deliveries list it: where it is in its
/// retry cycle and the last answer.
#[utoipa::path(
    get,
    tag = "Webhooks",
    path = "/api/v2/webhooks/deliveries/{id}",
    params(("id" = String, Path, description = "Webhook delivery id")),
    responses(
        (status = 200, description = "The delivery", body = WebhookDelivery),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_webhook_delivery(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WebhookDelivery>, Problem> {
    let details = find_delivery(&state, auth.tenant_id, &id).await?;
    Ok(Json(delivery(details.delivery, details.message.event_type)))
}

/// Resend a webhook delivery
///
/// Queues the same event for the same endpoint again, at once, as the same delivery (`manual`
/// from then on): its attempts are kept in its log, each with its answer, and attempt numbers go
/// on from the last. A delivery that had ended (succeeded, failed or cancelled) starts a new
/// retry budget: its `status`, `attempts`, `last_response_status` and `last_error` read the
/// resend's from then on. Refused while the endpoint is disabled, or while the delivery is being
/// sent.
#[utoipa::path(
    post,
    tag = "Webhooks",
    path = "/api/v2/webhooks/deliveries/{id}/resend",
    params(("id" = String, Path, description = "Webhook delivery id")),
    responses(
        (status = 200, description = "Queued again", body = WebhookDelivery),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The endpoint is disabled, or the delivery is being sent", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn resend_webhook_delivery(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WebhookDelivery>, Problem> {
    let details = find_delivery(&state, auth.tenant_id, &id).await?;
    let resent = state
        .store
        .resend_webhook_delivery(auth.tenant_id, details.delivery.id)
        .await?;
    Ok(Json(delivery(resent, details.message.event_type)))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_webhook_endpoints, create_webhook_endpoint))
        .routes(routes!(get_webhook_endpoint, update_webhook_endpoint))
        .routes(routes!(archive_webhook_endpoint))
        .routes(routes!(get_webhook_endpoint_secret))
        .routes(routes!(rotate_webhook_endpoint_secret))
        .routes(routes!(list_webhook_deliveries))
        .routes(routes!(get_webhook_delivery))
        .routes(routes!(resend_webhook_delivery))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_types_read_as_a_payload_names_them() {
        let asked = vec![
            "invoice.paid".to_owned(),
            " subscription.created ".to_owned(),
        ];
        assert_eq!(
            event_types(&asked, "/event_types").unwrap(),
            vec!["invoice.paid", "subscription.created"]
        );
        let unknown = event_types(&["INVOICE_PAID".to_owned()], "/event_types").unwrap_err();
        assert_eq!(unknown.errors[0].path, "/event_types/0");
    }

    #[test]
    fn an_endpoint_is_pinned_to_a_published_version() {
        assert_eq!(
            api_version(Some(" 2026-12-01 ".to_owned())).unwrap(),
            "2026-12-01"
        );
        for refused in [Some("2027-01-01".to_owned()), Some("v1".to_owned()), None] {
            let problem = api_version(refused).unwrap_err();
            assert_eq!(problem.errors[0].path, "/api_version");
        }
    }

    /// The verification the document describes is the one deliveries are signed with.
    #[test]
    fn the_signature_described_is_the_one_sent() {
        use meteroid_store::repositories::webhooks_out::PREVIOUS_SECRET_GRACE;
        use meteroid_store::services::webhooks::out::secret::SECRET_PREFIX;
        use meteroid_store::services::webhooks::out::signing;
        for named in [
            signing::HEADER_ID,
            signing::HEADER_TIMESTAMP,
            signing::HEADER_SIGNATURE,
            signing::HEADER_ID_LEGACY,
            signing::HEADER_TIMESTAMP_LEGACY,
            signing::HEADER_SIGNATURE_LEGACY,
            SECRET_PREFIX,
        ] {
            assert!(
                SIGNATURE_VERIFICATION.contains(&format!("`{named}")),
                "{named}"
            );
        }
        assert!(
            SIGNATURE_VERIFICATION
                .contains(&format!("{} hours", PREVIOUS_SECRET_GRACE.num_hours()))
        );
        let signature = signing::sign(
            &secrecy::SecretString::from("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"),
            "evt_1",
            1_700_000_000,
            b"{}",
        )
        .unwrap();
        assert!(signature.starts_with("v1,") && SIGNATURE_VERIFICATION.contains("`v1,`"));
    }

    #[test]
    fn statuses_map_both_ways() {
        for status in [
            DeliveryStatus::Pending,
            DeliveryStatus::InFlight,
            DeliveryStatus::Succeeded,
            DeliveryStatus::Failed,
            DeliveryStatus::Cancelled,
        ] {
            assert_eq!(
                DeliveryStatus::from(DomainDeliveryStatus::from(status)),
                status
            );
        }
        assert_eq!(
            serde_json::to_value(DeliveryStatus::InFlight).unwrap(),
            "in_flight"
        );
        assert_eq!(
            serde_json::to_value(DisabledReason::AutoFailures).unwrap(),
            "auto_failures"
        );
    }
}
