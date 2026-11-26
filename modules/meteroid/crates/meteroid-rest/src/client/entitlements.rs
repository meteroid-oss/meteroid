//! What the token's customer may use, read as API v2 reads it (ADR-0013 §23): the snapshot with
//! its `ETag`, and the check. A token names only the resources and attributes its claims allow;
//! `explain` and `current_count` take a secret key.
use crate::AppState;
use crate::client::AuthorizedAsCustomer;
use crate::entitlements::mapping::bracketed_params;
use crate::v2::customers::checks::{
    CheckAsked, DROPPED_INPUTS, EntitlementCheck, check, decimal, refuse_dropped,
};
use crate::v2::customers::entitlements::{EntitlementSnapshot, query_input, snapshot_response};
use crate::v2::customers::resources::{ResourceName, query_resource, resource_input};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::problem::{Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::{RawQuery, State};
use axum::response::Response;
use http::{HeaderMap, StatusCode};
use meteroid_store::domain::resources::ResourceInput;
use meteroid_store::repositories::ResourcesInterface;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

fn forbidden(detail: &str) -> Problem {
    Problem::new(StatusCode::FORBIDDEN, ProblemCode::Forbidden, detail)
}

/// What a check asks through the client API, as API v2's check asks it. `explain` and
/// `current_count` take a secret key.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[schema(as = ClientCheckEntitlement)]
pub struct ClientCheckEntitlement {
    /// The feature, by code or id.
    pub feature: String,
    /// The resource the token claims, `{"id": …}` or `{"type": …, "key": …}`; left out, the
    /// customer as a whole.
    #[serde(default)]
    pub resource: Option<ResourceName>,
    /// Attributes the token claims, `{"site.environment": "staging"}`.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// The units about to be used; 1 by default.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub requested: Option<Decimal>,
    /// Refused: it takes a secret key.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub current_count: Option<Decimal>,
    /// Refused: it takes a secret key.
    #[serde(default)]
    pub explain: bool,
}

/// The query of the client check's `GET`; `resource` (or `resource[type]` and `resource[key]`)
/// and `attributes[<type>.<attribute>]=<value>` are read beside it.
#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct ClientCheckQuery {
    /// The units about to be used, a decimal string; 1 by default.
    pub requested: Option<String>,
    /// Refused: it takes a secret key.
    pub current_count: Option<String>,
    /// Refused: it takes a secret key.
    #[serde(default)]
    pub explain: bool,
}

/// A request naming only what the token claims runs with all of its claims.
async fn claimed(
    state: &AppState,
    auth: &AuthorizedAsCustomer,
    named: &ResourceInput,
) -> Result<ResourceInput, Problem> {
    let types = if named.resources.is_empty() {
        vec![]
    } else {
        state
            .store
            .list_resource_types(auth.tenant_id, true)
            .await?
    };
    if auth.claims.allows(named, &types) {
        Ok(auth.claims.input())
    } else {
        Err(forbidden(
            "The token does not claim the resource or the attributes named",
        ))
    }
}

/// The check with its resource resolved and the token's claims applied; `pointer` prefixes the
/// body's fields (`/`), or is empty for the query's parameters.
async fn authorized_check(
    state: &AppState,
    auth: &AuthorizedAsCustomer,
    request: ClientCheckEntitlement,
    pointer: &str,
) -> Result<CheckAsked, Problem> {
    if request.explain || request.current_count.is_some() {
        return Err(forbidden("`explain` and `current_count` take a secret key"));
    }
    let named = resource_input(
        state,
        auth.tenant_id,
        auth.customer_id,
        request.resource.as_ref(),
        request.attributes,
        &format!("{pointer}resource"),
    )
    .await?;
    Ok(CheckAsked {
        feature: request.feature,
        input: claimed(state, auth, &named).await?,
        requested: request.requested,
        current_count: request.current_count,
        explain: request.explain,
    })
}

/// Get entitlements
///
/// What the customer may use, for the customer as a whole and for the resource the token
/// claims, as the server API's `GET /api/v2/customers/{id_or_alias}/entitlements` answers it,
/// with its `ETag`.
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/client/v1/entitlements",
    operation_id = "get_entitlements",
    params(
        ("resource" = Option<String>, Query, description = "A stored resource the token claims, by id"),
        ("resource[type]" = Option<String>, Query, description = "With `resource[key]`: a resource the token claims, by its type's code"),
        ("resource[key]" = Option<String>, Query, description = "With `resource[type]`: the resource's key value"),
        ("attributes" = Option<BTreeMap<String, String>>, Query, style = DeepObject, explode, description = "Attributes the token claims, as `attributes[<type>.<attribute>]=<value>`"),
        ("If-None-Match" = Option<String>, Header, description = "The `ETag` of a previous answer"),
    ),
    responses(
        (status = 200, description = "The snapshot", body = EntitlementSnapshot),
        (status = 304, description = "Unchanged since the `If-None-Match` etag"),
        (status = 401, description = "Missing, invalid or expired token (`token_expired`)", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not grant read access, or does not claim the resource or the attributes", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A resource not the customer's or of no type, or an unknown attribute or value", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The snapshot could not be answered in time; retry", body = Problem, content_type = "application/problem+json"),
    ),
    security(("customer_token" = []))
)]
pub async fn get_entitlements(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, Problem> {
    let named = query_input(&state, auth.tenant_id, auth.customer_id, query.as_deref()).await?;
    let input = claimed(&state, &auth, &named).await?;
    snapshot_response(
        &state,
        auth.tenant_id,
        auth.customer_id,
        input,
        &headers,
        "resource",
    )
    .await
}

/// Check an entitlement
///
/// Whether the customer may use `requested` more of a feature now, as the server API's `POST
/// /api/v2/customers/{id_or_alias}/entitlements/check` answers; `explain` and `current_count`
/// are refused.
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/client/v1/entitlements/check",
    operation_id = "check_entitlement",
    request_body = ClientCheckEntitlement,
    responses(
        (status = 200, description = "Whether the customer may use the requested units", body = EntitlementCheck),
        (status = 401, description = "Missing, invalid or expired token (`token_expired`)", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not claim the resource or the attributes, or the request asks for `explain` or `current_count`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "The feature is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A negative `requested`, a resource not the customer's or of no type, or an unknown attribute or value", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The check could not be answered in time; retry", body = Problem, content_type = "application/problem+json"),
    ),
    security(("customer_token" = []))
)]
pub async fn check_entitlement(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(state): State<AppState>,
    Body(body): Body<ClientCheckEntitlement>,
) -> Result<Json<EntitlementCheck>, Problem> {
    let request = authorized_check(&state, &auth, body, "/").await?;
    let answer = check(
        &state,
        auth.tenant_id,
        auth.customer_id,
        request,
        None,
        "/",
        "resource",
    )
    .await?;
    Ok(Json(answer))
}

/// Check an entitlement (GET)
///
/// The `GET` form of `POST /api/client/v1/entitlements/check`: the feature by code or id, its
/// inputs as query parameters: `resource` (an id) or `resource[type]` and `resource[key]`, and
/// `attributes[<type>.<attribute>]=<value>`.
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/client/v1/entitlements/{feature}/check",
    operation_id = "check_entitlement_by_query",
    params(
        ("feature" = String, Path, description = "Feature code or id"),
        ClientCheckQuery,
        ("resource" = Option<String>, Query, description = "A stored resource the token claims, by id"),
        ("resource[type]" = Option<String>, Query, description = "With `resource[key]`: a resource the token claims, by its type's code"),
        ("resource[key]" = Option<String>, Query, description = "With `resource[type]`: the resource's key value"),
        ("attributes" = Option<BTreeMap<String, String>>, Query, style = DeepObject, explode, description = "Attributes the token claims, as `attributes[<type>.<attribute>]=<value>`"),
    ),
    responses(
        (status = 200, description = "Whether the customer may use the requested units", body = EntitlementCheck),
        (status = 401, description = "Missing, invalid or expired token (`token_expired`)", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not claim the resource or the attributes, or the request asks for `explain` or `current_count`", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "The feature is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The check could not be answered in time; retry", body = Problem, content_type = "application/problem+json"),
    ),
    security(("customer_token" = []))
)]
pub async fn check_entitlement_by_query(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(state): State<AppState>,
    Path(feature): Path<String>,
    Query(query): Query<ClientCheckQuery>,
    RawQuery(raw): RawQuery,
) -> Result<Json<EntitlementCheck>, Problem> {
    refuse_dropped(raw.as_deref(), DROPPED_INPUTS)?;
    let request = ClientCheckEntitlement {
        feature,
        resource: query_resource(raw.as_deref())?,
        attributes: bracketed_params(raw.as_deref(), "attributes"),
        requested: decimal(query.requested.as_deref(), "requested")?,
        current_count: decimal(query.current_count.as_deref(), "current_count")?,
        explain: query.explain,
    };
    let request = authorized_check(&state, &auth, request, "").await?;
    let answer = check(
        &state,
        auth.tenant_id,
        auth.customer_id,
        request,
        None,
        "",
        "resource",
    )
    .await?;
    Ok(Json(answer))
}
