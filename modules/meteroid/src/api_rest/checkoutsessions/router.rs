use axum::extract::{Path, State};
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{CheckoutSessionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;

use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::subscriptions::mapping as sub_mapping;
use crate::api_rest::{AppState, QueryParams};
use crate::errors::RestApiError;
use meteroid_store::domain::SubscriptionActivationCondition;
use meteroid_store::domain::checkout_sessions::{
    CheckoutPaymentStrategy, CheckoutType, CreateCheckoutSession,
};
use meteroid_store::jwt_claims::{ResourceAccess, generate_portal_token};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

use super::mapping;
use super::model::{
    CancelCheckoutSessionResponse, CreateCheckoutSessionRequest, CreateCheckoutSessionResponse,
    GetCheckoutSessionResponse, ListCheckoutSessionsQuery, ListCheckoutSessionsResponse,
    PaymentStrategy,
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
    )
)]
pub async fn create_checkout_session(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Json(request): Json<CreateCheckoutSessionRequest>,
) -> Result<Json<CreateCheckoutSessionResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    // Default 1 hour
    let expires_in_hours = request.expires_in_hours.unwrap_or(1);

    let expires_in_hours = if expires_in_hours == 0 {
        None
    } else {
        Some(expires_in_hours)
    };

    let activation_condition = request
        .activation_condition
        .map(Into::into)
        .unwrap_or(SubscriptionActivationCondition::OnStart);

    let payment_strategy = request.payment_strategy.map(map_payment_strategy);

    let components = request.components.map(Into::into);

    let add_ons = request.add_ons.map(sub_mapping::map_add_ons);

    let create_session = CreateCheckoutSession {
        tenant_id,
        customer_id: request.customer_id,
        plan_version_id: request.plan_version_id,
        created_by: authorized_state.actor_id,
        billing_start_date: request.billing_start_date,
        billing_day_anchor: request.billing_day_anchor.map(|a| a as i16),
        net_terms: request.net_terms,
        trial_duration_days: request.trial_duration_days,
        end_date: request.end_date,
        activation_condition,
        auto_advance_invoices: request.auto_advance_invoices.unwrap_or(true),
        charge_automatically: request.charge_automatically.unwrap_or(true),
        invoice_memo: request.invoice_memo,
        invoice_threshold: request.invoice_threshold,
        purchase_order: request.purchase_order,
        payment_strategy,
        components,
        add_ons,
        coupon_code: request.coupon_code,
        coupon_ids: request.coupon_ids,
        expires_in_hours,
        metadata: request.metadata,
        checkout_type: CheckoutType::SelfServe,
        subscription_id: None,
    };

    let session = app_state
        .store
        .create_checkout_session(create_session)
        .await?;

    let checkout_url = generate_checkout_url(
        &app_state.jwt_secret,
        &app_state.portal_url,
        tenant_id,
        session.id,
    )?;

    let rest_session = mapping::domain_to_rest(session, Some(checkout_url));

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
        ("id" = String, Path, description = "Checkout session ID")
    ),
    responses(
        (status = 200, description = "Checkout session details", body = GetCheckoutSessionResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal server error", body = RestErrorResponse)
    )
)]
pub async fn get_checkout_session(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(session_id): Path<CheckoutSessionId>,
) -> Result<Json<GetCheckoutSessionResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    let session = app_state
        .store
        .get_checkout_session(tenant_id, session_id)
        .await?;

    let checkout_url = if session.can_complete() {
        Some(generate_checkout_url(
            &app_state.jwt_secret,
            &app_state.portal_url,
            tenant_id,
            session.id,
        )?)
    } else {
        None
    };

    let rest_session = mapping::domain_to_rest(session, checkout_url);

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
    )
)]
pub async fn list_checkout_sessions(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(QueryParams(query)): Valid<QueryParams<ListCheckoutSessionsQuery>>,
) -> Result<Json<ListCheckoutSessionsResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    let domain_status = query.status.map(mapping::status_rest_to_domain);

    let sessions = app_state
        .store
        .list_checkout_sessions(tenant_id, query.customer_id, domain_status)
        .await?;

    let rest_sessions = sessions
        .into_iter()
        .map(|session| {
            let checkout_url = if session.can_complete() {
                generate_checkout_url(
                    &app_state.jwt_secret,
                    &app_state.portal_url,
                    tenant_id,
                    session.id,
                )
                .ok()
            } else {
                None
            };
            mapping::domain_to_rest(session, checkout_url)
        })
        .collect();

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
        ("id" = String, Path, description = "Checkout session ID")
    ),
    responses(
        (status = 200, description = "Checkout session cancelled", body = CancelCheckoutSessionResponse),
        (status = 404, description = "Not found", body = RestErrorResponse),
        (status = 400, description = "Bad request - session cannot be cancelled", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal server error", body = RestErrorResponse)
    )
)]
pub async fn cancel_checkout_session(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(session_id): Path<CheckoutSessionId>,
) -> Result<Json<CancelCheckoutSessionResponse>, RestApiError> {
    let tenant_id = authorized_state.tenant_id;

    let session = app_state
        .store
        .cancel_checkout_session(tenant_id, session_id)
        .await?;

    let rest_session = mapping::domain_to_rest(session, None);

    Ok(Json(CancelCheckoutSessionResponse {
        session: rest_session,
    }))
}

fn generate_checkout_url(
    jwt_secret: &secrecy::SecretString,
    portal_url: &str,
    tenant_id: TenantId,
    session_id: CheckoutSessionId,
) -> Result<String, RestApiError> {
    let token = generate_portal_token(
        jwt_secret,
        tenant_id,
        ResourceAccess::CheckoutSession(session_id),
    )
    .map_err(|_| RestApiError::StoreError)?;

    Ok(format!("{}/checkout?token={}", portal_url, token))
}

fn map_payment_strategy(ps: PaymentStrategy) -> CheckoutPaymentStrategy {
    match ps {
        PaymentStrategy::Auto => CheckoutPaymentStrategy::Auto,
        PaymentStrategy::Bank => CheckoutPaymentStrategy::Bank,
        PaymentStrategy::External => CheckoutPaymentStrategy::External,
    }
}
