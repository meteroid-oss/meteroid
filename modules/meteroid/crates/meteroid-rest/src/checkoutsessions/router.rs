use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{AliasOr, CheckoutSessionId, CustomerId, PlanOptionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_rest_common::extract::{ApiJson, ApiPath, ApiValid};
use std::str::FromStr;

use crate::AppState;
use crate::subscriptions::mapping as sub_mapping;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_store::domain::checkout_sessions::{CheckoutType, CreateCheckoutSession};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::repositories::{CustomersInterface, PlanOptionInterface};

use super::mapping;
use super::model::{
    CancelCheckoutSessionResponse, CreateCheckoutSessionRequest, CreateCheckoutSessionResponse,
    GetCheckoutSessionResponse, ListCheckoutSessionsQuery, ListCheckoutSessionsResponse,
};

/// Create a checkout session
#[utoipa::path(
    post,
    path = "/api/v1/checkout-sessions",
    tag = "Checkout Sessions",
    request_body = CreateCheckoutSessionRequest,
    responses(
        (status = 200, description = "Checkout session created", body = CreateCheckoutSessionResponse),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal server error", body = RestErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn create_checkout_session(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiJson(request): ApiJson<CreateCheckoutSessionRequest>,
) -> Result<Json<CreateCheckoutSessionResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    // Resolve customer ID or alias
    let id_or_alias: AliasOr<CustomerId> =
        AliasOr::from_str(&request.customer_id).map_err(|_| {
            log::error!("Invalid customer_id format: {}", request.customer_id);
            RestApiError::InvalidInput("invalid customer_id format".to_string())
        })?;

    let customer_id = match id_or_alias {
        AliasOr::Id(id) => id,
        AliasOr::Alias(ref alias) => {
            app_state
                .store
                .find_customer_id_by_alias(alias.clone(), tenant_id)
                .await
                .map_err(|e| {
                    log::error!("Failed to resolve customer alias '{}': {:?}", alias, e);
                    RestApiError::from(e)
                })?
                .id
        }
    };

    // Kept as the option's alias, valid across versions; an id is read as its option's alias.
    let option = match request
        .option
        .as_deref()
        .map(str::trim)
        .filter(|option| !option.is_empty())
    {
        None => None,
        Some(asked) => match AliasOr::<PlanOptionId>::from_str(asked) {
            Ok(AliasOr::Id(id)) => Some(
                app_state
                    .store
                    .list_plan_options(tenant_id, request.plan_version_id)
                    .await
                    .map_err(RestApiError::from)?
                    .into_iter()
                    .find(|option| option.id == id)
                    .ok_or_else(|| {
                        RestApiError::InvalidInput(format!("The plan has no option {asked}"))
                    })?
                    .alias,
            ),
            _ => Some(asked.to_string()),
        },
    };

    validate_redirect_url("success_url", request.success_url.as_deref())?;
    validate_redirect_url("cancel_url", request.cancel_url.as_deref())?;

    // Default 1 hour
    let expires_in_hours = request.expires_in_hours.unwrap_or(1);

    let expires_in_hours = if expires_in_hours == 0 {
        None
    } else {
        Some(expires_in_hours)
    };

    let components = request.components.map(Into::into);

    let add_ons = request.add_ons.map(sub_mapping::map_add_ons);

    let create_session = CreateCheckoutSession {
        tenant_id,
        customer_id,
        plan_version_id: request.plan_version_id,
        option,
        billing_start_date: request.billing_start_date,
        billing_day_anchor: request.billing_day_anchor.map(|a| a as i16),
        net_terms: request.net_terms,
        trial_duration_days: request.trial_duration_days,
        end_date: request.end_date,
        auto_advance_invoices: request.auto_advance_invoices.unwrap_or(true),
        charge_automatically: request.charge_automatically.unwrap_or(true),
        invoice_memo: request.invoice_memo,
        invoice_threshold: request.invoice_threshold,
        purchase_order: request.purchase_order,
        payment_methods_config: request.payment_methods_config.map(Into::into),
        components,
        add_ons,
        coupon_code: request.coupon_code.filter(|code| !code.trim().is_empty()),
        coupon_ids: request.coupon_ids,
        expires_in_hours,
        metadata: request.metadata,
        checkout_type: CheckoutType::SelfServe,
        subscription_id: None,
        change_date: None,
        success_url: request.success_url,
        cancel_url: request.cancel_url,
    };

    let session = app_state
        .store
        .create_checkout_session(create_session)
        .await
        .map_err(|e| {
            log::error!("Failed to create checkout session: {:?}", e);
            RestApiError::from(e)
        })?;

    let checkout_url = generate_checkout_url(&app_state, tenant_id, session.id)
        .await
        .map_err(|e| {
            log::error!("Failed to generate checkout URL: {:?}", e);
            e
        })?;

    let rest_session = mapping::domain_to_rest(session, Some(checkout_url))?;

    Ok(Json(CreateCheckoutSessionResponse {
        session: rest_session,
    }))
}

/// Get a checkout session by ID
#[utoipa::path(
    get,
    path = "/api/v1/checkout-sessions/{id}",
    tag = "Checkout Sessions",
    params(
        ("id" = CheckoutSessionId, Path, description = "Checkout session ID")
    ),
    responses(
        (status = 200, description = "Checkout session details", body = GetCheckoutSessionResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal server error", body = RestErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_checkout_session(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(session_id): ApiPath<CheckoutSessionId>,
) -> Result<Json<GetCheckoutSessionResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    let session = app_state
        .store
        .get_checkout_session(tenant_id, session_id)
        .await?;

    let checkout_url = if session.can_complete() {
        Some(generate_checkout_url(&app_state, tenant_id, session.id).await?)
    } else {
        None
    };

    let rest_session = mapping::domain_to_rest(session, checkout_url)?;

    Ok(Json(GetCheckoutSessionResponse {
        session: rest_session,
    }))
}

/// List checkout sessions
#[utoipa::path(
    get,
    path = "/api/v1/checkout-sessions",
    tag = "Checkout Sessions",
    params(ListCheckoutSessionsQuery),
    responses(
        (status = 200, description = "List of checkout sessions", body = ListCheckoutSessionsResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal server error", body = RestErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_checkout_sessions(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(QueryParams(query)): ApiValid<QueryParams<ListCheckoutSessionsQuery>>,
) -> Result<Json<ListCheckoutSessionsResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    let domain_status = query.status.map(mapping::status_rest_to_domain);

    let sessions = app_state
        .store
        .list_checkout_sessions(tenant_id, query.customer_id, domain_status)
        .await?;

    let mut rest_sessions = Vec::with_capacity(sessions.len());
    for session in sessions
        .into_iter()
        .filter(|session| session.plan_version_id.is_some())
    {
        let checkout_url = if session.can_complete() {
            generate_checkout_url(&app_state, tenant_id, session.id)
                .await
                .ok()
        } else {
            None
        };
        rest_sessions.push(mapping::domain_to_rest(session, checkout_url)?);
    }

    Ok(Json(ListCheckoutSessionsResponse {
        sessions: rest_sessions,
    }))
}

/// Cancel a checkout session
#[utoipa::path(
    post,
    path = "/api/v1/checkout-sessions/{id}/cancel",
    tag = "Checkout Sessions",
    params(
        ("id" = CheckoutSessionId, Path, description = "Checkout session ID")
    ),
    responses(
        (status = 200, description = "Checkout session cancelled", body = CancelCheckoutSessionResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 400, description = "Bad request - session cannot be cancelled", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal server error", body = RestErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn cancel_checkout_session(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(session_id): ApiPath<CheckoutSessionId>,
) -> Result<Json<CancelCheckoutSessionResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    let session = app_state
        .store
        .cancel_checkout_session(tenant_id, session_id)
        .await?;

    let rest_session = mapping::domain_to_rest(session, None)?;

    Ok(Json(CancelCheckoutSessionResponse {
        session: rest_session,
    }))
}

/// Redirect targets are merchant-set and reflected to the customer's browser, so anything but an
/// absolute http(s) URL is rejected up front. API v2 checks with it too.
pub(crate) fn validate_redirect_url(field: &str, value: Option<&str>) -> Result<(), RestApiError> {
    let Some(value) = value else {
        return Ok(());
    };
    let is_http = url::Url::parse(value)
        .map(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
        .unwrap_or(false);
    if is_http {
        Ok(())
    } else {
        Err(RestApiError::InvalidInput(format!(
            "{field} must be an absolute http(s) URL"
        )))
    }
}

/// The hosted checkout page of the session; API v2 links it too.
pub(crate) async fn generate_checkout_url(
    app_state: &AppState,
    tenant_id: TenantId,
    session_id: CheckoutSessionId,
) -> Result<String, RestApiError> {
    let token = app_state
        .store
        .checkout_portal_token(tenant_id, session_id, false)
        .await
        .map_err(|_| RestApiError::StoreError)?;

    Ok(format!("{}/checkout?token={}", app_state.portal_url, token))
}
