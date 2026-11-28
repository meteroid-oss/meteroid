use crate::AppState;
use crate::ratecards::mapping::{
    card_to_rest, card_with_rates, create_req_to_domain, rates_req_to_domain,
};
use crate::ratecards::model::{
    RateCard, RateCardCreateRequest, RateCardGetRequest, RateCardListRequest, RateCardListResponse,
    RateCardRatesSetRequest, RateCardRatesSetResponse, RateCardWithRates,
};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use common_domain::ids::{AliasOr, RateCardId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::extract::{ApiJson, ApiPath, ApiQuery, ApiValid};
use meteroid_store::domain::rate_cards::RateCardPurpose;
use meteroid_store::repositories::RateCardsInterface;

fn today() -> chrono::NaiveDate {
    common_clock::now().date_naive()
}

/// A price card's id: a cost card is not found here.
async fn card_id(
    state: &AppState,
    tenant_id: TenantId,
    id_or_code: AliasOr<RateCardId>,
) -> Result<RateCardId, RestApiError> {
    let id = match id_or_code {
        AliasOr::Id(id) => id,
        AliasOr::Alias(code) => state
            .store
            .find_rate_card_id_by_code(tenant_id, &code)
            .await
            .map_err(RestApiError::from)?,
    };
    state
        .store
        .find_rate_card_of_purpose(tenant_id, id, RateCardPurpose::Price)
        .await
        .map_err(RestApiError::from)?;
    Ok(id)
}

/// List rate cards
///
/// The tenant's rate cards by code, archived ones left out: the rates of a metric's usage by
/// its dimensions (model, token type), that usage prices name.
#[utoipa::path(
    get,
    tag = "Rate cards",
    path = "/api/v1/rate-cards",
    params(RateCardListRequest),
    responses(
        (status = 200, description = "The tenant's rate cards", body = RateCardListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
        (status = 503, description = "Temporarily unavailable; retry shortly", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_rate_cards(
    Extension(auth): Extension<AuthorizedAsTenant>,
    ApiValid(ApiQuery(request)): ApiValid<ApiQuery<RateCardListRequest>>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let cards = state
        .store
        .list_rate_cards(
            auth.tenant_id,
            request.metric_id,
            Some(RateCardPurpose::Price),
        )
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(RateCardListResponse {
        data: cards.into_iter().map(card_to_rest).collect(),
    }))
}

/// Get a rate card
///
/// The card with the rate of each key and currency in force on `at` (today by default), and
/// the changes scheduled after it.
#[utoipa::path(
    get,
    tag = "Rate cards",
    path = "/api/v1/rate-cards/{id_or_code}",
    params(
        ("id_or_code" = String, Path, description = "Rate card ID or code"),
        RateCardGetRequest,
    ),
    responses(
        (status = 200, description = "The rate card and its rates", body = RateCardWithRates),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
        (status = 503, description = "Temporarily unavailable; retry shortly", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_rate_card(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiValid(ApiPath(id_or_code)): ApiValid<ApiPath<AliasOr<RateCardId>>>,
    ApiValid(ApiQuery(request)): ApiValid<ApiQuery<RateCardGetRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let id = card_id(&state, auth.tenant_id, id_or_code).await?;
    let (card, entries) = state
        .store
        .get_rate_card(auth.tenant_id, id)
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(card_with_rates(
        card,
        &entries,
        request.at.unwrap_or_else(today),
    )))
}

/// Create a rate card
///
/// A card rates one metric that adds up (a sum or a count, without a matrix) by one to four
/// dimensions. A `MONEY` card holds a rate per currency for each key, a price rated in its
/// currency's rates; a `CREDIT_UNIT` card rates in its credit unit. Set its rates with
/// `PATCH /api/v1/rate-cards/{id_or_code}/rates`.
#[utoipa::path(
    post,
    tag = "Rate cards",
    path = "/api/v1/rate-cards",
    request_body(content = RateCardCreateRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Created", body = RateCard),
        (status = 400, description = "Invalid request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "The metric or the credit unit is not found", body = RestErrorResponse),
        (status = 409, description = "A rate card with this code already exists", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
        (status = 503, description = "Temporarily unavailable; retry shortly", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_rate_card(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<RateCardCreateRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let new = create_req_to_domain(auth.tenant_id, payload)?;
    let card = state
        .store
        .create_rate_card(auth.as_actor(), new)
        .await
        .map_err(RestApiError::from)?;
    Ok((StatusCode::CREATED, Json(card_to_rest(card))))
}

/// Set a rate card's rates
///
/// Changes only the keys and currencies listed: sets their rates and removes keys, the whole
/// batch from `effective_from` (today by default). Keys not listed keep their rates.
/// Entries are never edited: a change is a new rate from its day, a removal ends the key in a
/// currency from its day (usage it rated falls to the next key matching it, or is unmatched),
/// and earlier days keep their rates. A key's first rate in a currency may start on a past day
/// unless another key rated its usage since; a change or a removal starts today or later. A
/// repeated batch changes nothing.
#[utoipa::path(
    patch,
    tag = "Rate cards",
    path = "/api/v1/rate-cards/{id_or_code}/rates",
    params(("id_or_code" = String, Path, description = "Rate card ID or code")),
    request_body(content = RateCardRatesSetRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "What the batch did, and the card as of its day", body = RateCardRatesSetResponse),
        (status = 400, description = "Invalid request: the batch is refused as a whole", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
        (status = 503, description = "Temporarily unavailable; retry shortly", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_rate_card_rates(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    ApiValid(ApiPath(id_or_code)): ApiValid<ApiPath<AliasOr<RateCardId>>>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<RateCardRatesSetRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let id = card_id(&state, auth.tenant_id, id_or_code).await?;
    let today = today();
    let at = payload.effective_from.unwrap_or(today);
    let summary = state
        .store
        .set_rate_card_rates(
            auth.as_actor(),
            auth.tenant_id,
            id,
            rates_req_to_domain(payload),
            today,
        )
        .await
        .map_err(RestApiError::from)?;
    let (card, entries) = state
        .store
        .get_rate_card(auth.tenant_id, id)
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(RateCardRatesSetResponse {
        inserted: summary.inserted,
        removed: summary.removed,
        unchanged: summary.unchanged,
        replaced: summary.replaced,
        rate_card: card_with_rates(card, &entries, at),
    }))
}
