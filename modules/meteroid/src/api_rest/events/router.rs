use crate::api_rest::AppState;
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::events::mapping;
use crate::api_rest::events::model::{IngestEventsRequest, IngestEventsResponse};
use crate::errors::RestApiError;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;

/// Ingest events
///
/// Ingest usage events for metering and billing purposes.
///
/// Events are deduplicated by `(event_id, customer_id)` — re-sending the same pair will not be
/// double-counted. If timestamps differ across duplicates, the event with the latest timestamp is used.
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
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn ingest_events(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(request)): Valid<Json<IngestEventsRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    // enterprise placeholder : ratelimit eps

    let usage_request = mapping::rest_request_to_usage_client(request);

    let response = app_state
        .services
        .usage_clients()
        .ingest_events(&authorized_state.tenant_id, usage_request)
        .await
        .map_err(RestApiError::from)?;

    let rest_response = mapping::usage_client_response_to_rest(response);

    Ok((StatusCode::OK, Json(rest_response)))
}
