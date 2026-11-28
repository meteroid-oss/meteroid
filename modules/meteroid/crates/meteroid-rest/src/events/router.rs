use crate::AppState;
use crate::events::mapping;
use crate::events::model::{IngestEventsRequest, IngestEventsResponse};
use crate::rate_limit::{RateLimiters, rate_limit_eps};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use metering_grpc::meteroid::metering::v1 as grpc;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::extract::{ApiJson, ApiValid};
use meteroid_store::clients::usage;
use meteroid_store::{Services, Store};

/// Ingest events
///
/// Ingest usage events for metering and billing purposes.
///
/// Events are deduplicated by `(event_id, customer_id)` — re-sending the same pair will not be
/// double-counted. The first copy received is kept and later copies are ignored, whatever they change.
///
/// By default, any invalid event rejects the entire batch. Set `allow_partial_failures` to `true` to ingest valid events and receive per-event failure details in the response body.
#[utoipa::path(
    post,
    tag = "Events",
    path = "/api/v1/events/ingest",
    request_body = IngestEventsRequest,
    responses(
        (status = 200, description = "Events ingested successfully", body = IngestEventsResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
        (status = 503, description = "Temporarily unavailable; retry shortly", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn ingest_events(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiJson(request)): ApiValid<ApiJson<IngestEventsRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let response = ingest(
        &app_state,
        &authorized_state,
        request
            .events
            .into_iter()
            .map(mapping::rest_event_to_grpc)
            .collect(),
        request.allow_backfilling.unwrap_or(false),
        !request.allow_partial_failures.unwrap_or(false),
    )
    .await?;
    Ok((
        StatusCode::OK,
        Json(mapping::usage_client_response_to_rest(response)),
    ))
}

/// Metering's ingestion of `events`, already checked, counted against the tenant's events per
/// second: v1's and v2's, each reading its own body into them.
pub(crate) async fn ingest(
    app_state: &AppState,
    authorized_state: &AuthorizedAsTenant,
    events: Vec<grpc::Event>,
    allow_backfilling: bool,
    fail_on_error: bool,
) -> Result<usage::IngestEventsResult, RestApiError> {
    ingest_checked(
        &app_state.store,
        &app_state.services,
        &app_state.rest_rate_limiters,
        authorized_state,
        events,
        allow_backfilling,
        fail_on_error,
    )
    .await
}

/// [`ingest`] for a module's endpoint, with its own limiters: built over the same backend and
/// buckets, they count against the tenant's one events-per-second quota.
pub async fn ingest_checked(
    store: &Store,
    services: &Services,
    limiters: &RateLimiters,
    authorized_state: &AuthorizedAsTenant,
    events: Vec<grpc::Event>,
    allow_backfilling: bool,
    fail_on_error: bool,
) -> Result<usage::IngestEventsResult, RestApiError> {
    rate_limit_eps(limiters, authorized_state, events.len()).await?;

    let keep_last_events = store
        .keep_last_events(authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;

    services
        .usage_clients()
        .ingest_events(
            &authorized_state.tenant_id,
            usage::IngestEventsRequest {
                events,
                allow_backfilling,
                fail_on_error,
                keep_last_events,
            },
        )
        .await
        .map_err(RestApiError::from)
}
