use crate::AppState;
use crate::client::AuthorizedAsCustomer;
use crate::client::mapping;
use crate::client::model::{ClientCustomer, ClientSubscriptionListResponse};
use crate::entitlements;
use crate::entitlements::model::EffectiveEntitlementListResponse;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::enums::SubscriptionStatusEnum;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;

/// Terminal subscriptions (cancelled, completed, superseded…) are left out.
const CURRENT_SUBSCRIPTION_STATUSES: [SubscriptionStatusEnum; 7] = [
    SubscriptionStatusEnum::PendingActivation,
    SubscriptionStatusEnum::PendingCharge,
    SubscriptionStatusEnum::TrialActive,
    SubscriptionStatusEnum::Active,
    SubscriptionStatusEnum::TrialExpired,
    SubscriptionStatusEnum::Paused,
    SubscriptionStatusEnum::Suspended,
];

const MAX_SUBSCRIPTIONS: u32 = 100;

/// Get the customer
///
/// The customer the token was minted for.
#[utoipa::path(
    get,
    tag = "Customer",
    path = "/api/client/v1/customer",
    responses(
        (status = 200, description = "Customer", body = ClientCustomer),
        (status = 401, description = "Missing, invalid or expired token", body = RestErrorResponse),
        (status = 403, description = "The token does not grant read access", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
    ),
    security(("customer_token" = []))
)]
#[axum::debug_handler]
pub async fn get_customer(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id(auth.customer_id, auth.tenant_id)
        .await
        .map_err(RestApiError::from)?;

    Ok(Json(mapping::customer_to_client(customer)))
}

/// List entitlements
///
/// The customer's effective entitlements, with live usage. Same schema as the server API's
/// customer entitlements.
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/client/v1/entitlements",
    responses(
        (status = 200, description = "Customer entitlements", body = EffectiveEntitlementListResponse),
        (status = 401, description = "Missing, invalid or expired token", body = RestErrorResponse),
        (status = 403, description = "The token does not grant read access", body = RestErrorResponse),
    ),
    security(("customer_token" = []))
)]
#[axum::debug_handler]
pub async fn list_entitlements(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let resolved = app_state
        .services
        .get_effective_entitlements(auth.customer_id, auth.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving customer entitlements: {e}");
            RestApiError::from(e)
        })?;

    let data = resolved
        .into_iter()
        .map(entitlements::mapping::effective_entitlement_to_rest)
        .collect();
    Ok(Json(EffectiveEntitlementListResponse { data }))
}

/// List subscriptions
///
/// The customer's current subscriptions, newest first. Cancelled, completed and superseded
/// subscriptions are left out.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/client/v1/subscriptions",
    responses(
        (status = 200, description = "Customer subscriptions", body = ClientSubscriptionListResponse),
        (status = 401, description = "Missing, invalid or expired token", body = RestErrorResponse),
        (status = 403, description = "The token does not grant read access", body = RestErrorResponse),
    ),
    security(("customer_token" = []))
)]
#[axum::debug_handler]
pub async fn list_subscriptions(
    Extension(auth): Extension<AuthorizedAsCustomer>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let subscriptions = app_state
        .store
        .list_subscriptions(
            auth.tenant_id,
            Some(auth.customer_id),
            None,
            Some(CURRENT_SUBSCRIPTION_STATUSES.to_vec()),
            None,
            vec![],
            None,
            PaginationRequest {
                page: 0,
                per_page: Some(MAX_SUBSCRIPTIONS),
            },
        )
        .await
        .map_err(RestApiError::from)?;

    let data = subscriptions
        .items
        .into_iter()
        .map(mapping::subscription_to_client)
        .collect();
    Ok(Json(ClientSubscriptionListResponse { data }))
}
