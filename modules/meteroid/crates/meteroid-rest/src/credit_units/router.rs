use crate::AppState;
use crate::credit_units::mapping::{
    create_req_to_domain, domain_to_rest, price_to_rest, update_req_to_domain,
};
use crate::credit_units::model::{
    CreditUnit, CreditUnitCreateRequest, CreditUnitListRequest, CreditUnitListResponse,
    CreditUnitPrice, CreditUnitPriceListRequest, CreditUnitPriceListResponse,
    CreditUnitPriceSetRequest, CreditUnitUpdateRequest,
};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{CreditUnitId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::rest::writes::store_error;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_store::domain::credit_units as domain;
use meteroid_store::repositories::CreditUnitsInterface;

/// The unit with its active selling prices.
async fn with_prices(
    state: &AppState,
    tenant_id: TenantId,
    unit: domain::CreditUnit,
) -> Result<CreditUnit, RestApiError> {
    let prices = state
        .store
        .list_credit_unit_prices(tenant_id, vec![unit.id], false)
        .await
        .map_err(RestApiError::from)?;
    Ok(domain_to_rest(unit, &prices))
}

/// List credit units
///
/// The tenant's credit units (e.g. "Halyard credits"): denominations usage can be priced and
/// credit granted in, instead of money, each with its selling prices.
#[utoipa::path(
    get,
    tag = "Credit units",
    path = "/api/v1/credit-units",
    params(CreditUnitListRequest),
    responses(
        (status = 200, description = "The tenant's credit unit catalog", body = CreditUnitListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_credit_units(
    Extension(auth): Extension<AuthorizedAsTenant>,
    Valid(Query(request)): Valid<Query<CreditUnitListRequest>>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let units = state
        .store
        .list_credit_units(auth.tenant_id, request.include_archived.unwrap_or(false))
        .await
        .map_err(RestApiError::from)?;
    let prices = state
        .store
        .list_credit_unit_prices(
            auth.tenant_id,
            units.iter().map(|unit| unit.id).collect(),
            false,
        )
        .await
        .map_err(RestApiError::from)?;

    Ok(Json(CreditUnitListResponse {
        data: units
            .into_iter()
            .map(|unit| domain_to_rest(unit, &prices))
            .collect(),
    }))
}

/// Get a credit unit
///
/// The unit with its active selling prices.
#[utoipa::path(
    get,
    tag = "Credit units",
    path = "/api/v1/credit-units/{id}",
    params(("id" = String, Path, description = "Credit unit ID")),
    responses(
        (status = 200, description = "The credit unit", body = CreditUnit),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<CreditUnitId>,
) -> Result<impl IntoResponse, RestApiError> {
    let unit = state
        .store
        .get_credit_unit(auth.tenant_id, id)
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(with_prices(&state, auth.tenant_id, unit).await?))
}

/// Create a credit unit
///
/// `code` and `precision` are immutable once created. Price usage in the unit with a plan fee's
/// `credit_unit`, grant it with a credit pack or a credit grant, and sell it with a top-up once
/// it has a `product_id` and a selling price.
#[utoipa::path(
    post,
    tag = "Credit units",
    path = "/api/v1/credit-units",
    request_body(content = CreditUnitCreateRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Created", body = CreditUnit),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 409, description = "A credit unit with this code already exists", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreditUnitCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let new = create_req_to_domain(auth.tenant_id, payload)?;
    let created = state
        .store
        .create_credit_unit(new)
        .await
        .map_err(RestApiError::from)?;

    Ok((
        StatusCode::CREATED,
        Json(with_prices(&state, auth.tenant_id, created).await?),
    ))
}

/// Update a credit unit
///
/// Partial update: an absent field is left unchanged. `code` and `precision` are immutable and
/// cannot be changed here. Setting `archived: true` blocks the unit from new prices, packs and
/// grants; existing ones keep working.
#[utoipa::path(
    patch,
    tag = "Credit units",
    path = "/api/v1/credit-units/{id}",
    params(("id" = String, Path, description = "Credit unit ID")),
    request_body(content = CreditUnitUpdateRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Updated", body = CreditUnit),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 409, description = "The product is archived", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<CreditUnitId>,
    Valid(Json(payload)): Valid<Json<CreditUnitUpdateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let patch = update_req_to_domain(auth.tenant_id, id, payload)?;
    let unit = state
        .store
        .update_credit_unit(patch)
        .await
        .map_err(store_error)?;
    Ok(Json(with_prices(&state, auth.tenant_id, unit).await?))
}

/// List a credit unit's selling prices
///
/// The active price per currency, and archived ones when asked, newest first.
#[utoipa::path(
    get,
    tag = "Credit units",
    path = "/api/v1/credit-units/{id}/prices",
    params(("id" = String, Path, description = "Credit unit ID"), CreditUnitPriceListRequest),
    responses(
        (status = 200, description = "The unit's selling prices", body = CreditUnitPriceListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_credit_unit_prices(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<CreditUnitId>,
    Valid(Query(request)): Valid<Query<CreditUnitPriceListRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    state
        .store
        .get_credit_unit(auth.tenant_id, id)
        .await
        .map_err(RestApiError::from)?;
    let prices = state
        .store
        .list_credit_unit_prices(
            auth.tenant_id,
            vec![id],
            request.include_archived.unwrap_or(false),
        )
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(CreditUnitPriceListResponse {
        data: prices.iter().map(price_to_rest).collect(),
    }))
}

/// Set a credit unit's selling price
///
/// The price of one credit in a currency, used to sell the unit through top-ups and automatic
/// top-ups (taxed at purchase under the unit's `product_id`). Replaces the active price in that
/// currency, which is archived: a top-up already priced keeps its terms.
#[utoipa::path(
    put,
    tag = "Credit units",
    path = "/api/v1/credit-units/{id}/prices/{currency}",
    params(
        ("id" = String, Path, description = "Credit unit ID"),
        ("currency" = String, Path, description = "ISO 4217 currency code"),
    ),
    request_body(content = CreditUnitPriceSetRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The active price", body = CreditUnitPrice),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 409, description = "The credit unit is archived", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_credit_unit_price(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id, currency)): Path<(CreditUnitId, String)>,
    Valid(Json(payload)): Valid<Json<CreditUnitPriceSetRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let price = state
        .store
        .set_credit_unit_price(auth.tenant_id, id, &currency, payload.unit_price)
        .await
        .map_err(store_error)?;
    Ok(Json(price_to_rest(&price)))
}

/// Remove a credit unit's selling price
///
/// Archives the active price in a currency: the unit can no longer be sold in it (top-ups and
/// automatic top-ups included) until a new price is set.
#[utoipa::path(
    delete,
    tag = "Credit units",
    path = "/api/v1/credit-units/{id}/prices/{currency}",
    params(
        ("id" = String, Path, description = "Credit unit ID"),
        ("currency" = String, Path, description = "ISO 4217 currency code"),
    ),
    responses(
        (status = 200, description = "The archived price", body = CreditUnitPrice),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "No active price in this currency", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_credit_unit_price(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id, currency)): Path<(CreditUnitId, String)>,
) -> Result<impl IntoResponse, RestApiError> {
    let currency = currency.trim().to_ascii_uppercase();
    let active = state
        .store
        .list_credit_unit_prices(auth.tenant_id, vec![id], false)
        .await
        .map_err(RestApiError::from)?
        .into_iter()
        .find(|price| price.currency == currency)
        .ok_or(RestApiError::NotFound)?;
    let price = state
        .store
        .archive_credit_unit_price(auth.tenant_id, id, active.id)
        .await
        .map_err(store_error)?;
    Ok(Json(price_to_rest(&price)))
}
