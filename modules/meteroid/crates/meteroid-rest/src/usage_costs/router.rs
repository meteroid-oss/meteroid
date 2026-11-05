use super::model::{
    ListUsageCostsRequest, ListUsageEventsRequest, UsageCostListResponse, UsageEventListResponse,
    decode_cursor, encode_cursor,
};
use crate::AppState;
use crate::credits::router::resolve_customer;
use crate::rate_limit::rate_limit_usage_costs;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{AliasOr, BillableMetricId, CustomerId, SubscriptionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_app::errors::RestApiError;
use meteroid_app::features::gates;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_store::domain::usage_costs::{MetricRef, UsageOwner};

const DEFAULT_EVENTS_LIMIT: u32 = 100;

fn metric_ref(raw: &str) -> MetricRef {
    raw.parse::<BillableMetricId>()
        .map_or_else(|_| MetricRef::Code(raw.to_string()), MetricRef::Id)
}

async fn usage_events(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    owner: UsageOwner,
    request: ListUsageEventsRequest,
) -> Result<Json<UsageEventListResponse>, RestApiError> {
    let after = request
        .cursor
        .as_deref()
        .map(|raw| {
            decode_cursor(raw).ok_or_else(|| RestApiError::InvalidInput("invalid cursor".into()))
        })
        .transpose()?;

    let page = state
        .services
        .list_metric_events_with_cost(
            auth.tenant_id,
            owner,
            metric_ref(&request.metric),
            request.from.naive_utc(),
            request.to.naive_utc(),
            after,
            request.limit.unwrap_or(DEFAULT_EVENTS_LIMIT),
        )
        .await
        .map_err(RestApiError::from)?;

    Ok(Json(UsageEventListResponse {
        data: page.events.into_iter().map(Into::into).collect(),
        next_cursor: page.next.as_ref().map(encode_cursor),
    }))
}

async fn usage_costs(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    owner: UsageOwner,
    request: ListUsageCostsRequest,
) -> Result<Json<UsageCostListResponse>, RestApiError> {
    let rows = state
        .services
        .usage_costs(
            auth.tenant_id,
            owner,
            metric_ref(&request.metric),
            request.group_by,
            request.granularity.into(),
            request.from.naive_utc(),
            request.to.naive_utc(),
        )
        .await
        .map_err(RestApiError::from)?;

    Ok(Json(UsageCostListResponse {
        data: rows.into_iter().map(Into::into).collect(),
    }))
}

/// List a customer's usage events with their cost
///
/// A customer's events for one billable metric, oldest first, each with the units it adds and what
/// they cost at the price in force. The cost is exact for per-unit and matrix pricing and absent
/// for pricing that depends on the whole period's usage (tiered, volume, package, capacity) and for
/// aggregations that cannot be split per event. The range is at most 93 days.
#[utoipa::path(
    get,
    tag = "Usage costs",
    path = "/api/v1/usage/customer/{customer_id}/events",
    params(
        ("customer_id" = String, Path, description = "Customer ID or alias"),
        ListUsageEventsRequest,
    ),
    responses(
        (status = 200, description = "A page of events", body = UsageEventListResponse),
        (status = 400, description = "Invalid request or range", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Usage costs are not enabled for this organization", body = RestErrorResponse),
        (status = 404, description = "Customer or billable metric not found", body = RestErrorResponse),
        (status = 429, description = "Too many usage cost reads", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_customer_usage_events(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(QueryParams(request)): Valid<QueryParams<ListUsageEventsRequest>>,
) -> Result<Json<UsageEventListResponse>, RestApiError> {
    ensure_usage_costs(&state, &auth).await?;
    rate_limit_usage_costs(&state, &auth).await?;
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    usage_events(&state, &auth, UsageOwner::Customer(customer.id), request).await
}

/// List a subscription's usage events with their cost
///
/// The events of one billable metric that the subscription bills, oldest first: with a usage
/// filter, only the customer's events matching it; without, those no other subscription's filter
/// claims. Costs are as for the customer's events. The range is at most 93 days.
#[utoipa::path(
    get,
    tag = "Usage costs",
    path = "/api/v1/usage/subscription/{subscription_id}/events",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID"),
        ListUsageEventsRequest,
    ),
    responses(
        (status = 200, description = "A page of events", body = UsageEventListResponse),
        (status = 400, description = "Invalid request or range, or a metric the subscription does not bill", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Usage costs are not enabled for this organization", body = RestErrorResponse),
        (status = 404, description = "Subscription or billable metric not found", body = RestErrorResponse),
        (status = 429, description = "Too many usage cost reads", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_subscription_usage_events(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(QueryParams(request)): Valid<QueryParams<ListUsageEventsRequest>>,
) -> Result<Json<UsageEventListResponse>, RestApiError> {
    ensure_usage_costs(&state, &auth).await?;
    rate_limit_usage_costs(&state, &auth).await?;
    usage_events(
        &state,
        &auth,
        UsageOwner::Subscription(subscription_id),
        request,
    )
    .await
}

/// List a customer's usage costs over time
///
/// A customer's usage of one billable metric and what it costs, per hour, day or month, optionally
/// split by an event property, summed across the subscriptions that bill it. Buckets are cut at UTC
/// boundaries and only cover the customer's billing periods. Per-unit and matrix pricing give exact
/// costs. For pricing that depends on the whole period's usage (tiered, volume, package, capacity),
/// a bucket's cost is what it adds to the period's price so far and `exact` is false. The range is
/// at most 93 days.
#[utoipa::path(
    get,
    tag = "Usage costs",
    path = "/api/v1/usage/customer/{customer_id}/costs",
    params(
        ("customer_id" = String, Path, description = "Customer ID or alias"),
        ListUsageCostsRequest,
    ),
    responses(
        (status = 200, description = "Usage and cost per bucket", body = UsageCostListResponse),
        (status = 400, description = "Invalid request or range", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Usage costs are not enabled for this organization", body = RestErrorResponse),
        (status = 404, description = "Customer or billable metric not found", body = RestErrorResponse),
        (status = 429, description = "Too many usage cost reads", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_customer_usage_costs(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    Valid(QueryParams(request)): Valid<QueryParams<ListUsageCostsRequest>>,
) -> Result<Json<UsageCostListResponse>, RestApiError> {
    ensure_usage_costs(&state, &auth).await?;
    rate_limit_usage_costs(&state, &auth).await?;
    let customer = resolve_customer(&state, &auth, id_or_alias).await?;
    usage_costs(&state, &auth, UsageOwner::Customer(customer.id), request).await
}

/// List a subscription's usage costs over time
///
/// The subscription's usage of one billable metric and what it costs, as for a customer but
/// counting only the events the subscription bills (see its usage filter) and only its own billing
/// periods. The range is at most 93 days.
#[utoipa::path(
    get,
    tag = "Usage costs",
    path = "/api/v1/usage/subscription/{subscription_id}/costs",
    params(
        ("subscription_id" = SubscriptionId, Path, description = "Subscription ID"),
        ListUsageCostsRequest,
    ),
    responses(
        (status = 200, description = "Usage and cost per bucket", body = UsageCostListResponse),
        (status = 400, description = "Invalid request or range, or a metric the subscription does not bill", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Usage costs are not enabled for this organization", body = RestErrorResponse),
        (status = 404, description = "Subscription or billable metric not found", body = RestErrorResponse),
        (status = 429, description = "Too many usage cost reads", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_subscription_usage_costs(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(QueryParams(request)): Valid<QueryParams<ListUsageCostsRequest>>,
) -> Result<Json<UsageCostListResponse>, RestApiError> {
    ensure_usage_costs(&state, &auth).await?;
    rate_limit_usage_costs(&state, &auth).await?;
    usage_costs(
        &state,
        &auth,
        UsageOwner::Subscription(subscription_id),
        request,
    )
    .await
}

pub(crate) async fn usage_costs_open(state: &AppState, auth: &AuthorizedAsTenant) -> bool {
    state
        .feature_resolver
        .resolve(auth.organization_id)
        .await
        .is_enabled(gates::USAGE_COSTS)
}

async fn ensure_usage_costs(
    state: &AppState,
    auth: &AuthorizedAsTenant,
) -> Result<(), RestApiError> {
    if usage_costs_open(state, auth).await {
        Ok(())
    } else {
        Err(RestApiError::Forbidden)
    }
}
