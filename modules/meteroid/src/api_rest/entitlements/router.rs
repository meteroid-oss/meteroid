use super::AppState;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{
    AddOnId, AliasOr, CustomerId, EntitlementEntityId, FeatureId, PlanVersionId, ProductId,
    SubscriptionId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::entitlements::{EntitlementsInterface, ResolveTarget};
use meteroid_store::repositories::plans::PlansInterface;
use meteroid_store::repositories::quotes::QuotesInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterface;

use crate::api_rest::QueryParams;
use crate::api_rest::entitlements::mapping;
use crate::api_rest::entitlements::model::{
    EffectiveEntitlementListResponse, EntitlementListResponse, Feature, FeatureListRequest,
    FeatureListResponse, ResolvedEntitlementListResponse,
};
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;

/// List features
#[utoipa::path(
    get,
    tag = "Features",
    path = "/api/v1/features",
    params(FeatureListRequest),
    responses(
        (status = 200, description = "List of features", body = FeatureListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_features(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<FeatureListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let statuses = if request.statuses.is_empty() {
        None
    } else {
        Some(
            request
                .statuses
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>(),
        )
    };
    let res = app_state
        .store
        .list_features(
            authorized_state.tenant_id,
            request.pagination.into(),
            statuses,
            request.product_id,
            request.search,
        )
        .await
        .map_err(|e| {
            log::error!("Error listing features: {e}");
            RestApiError::StoreError
        })?;

    let data = res
        .items
        .into_iter()
        .map(mapping::feature_to_rest)
        .collect();

    Ok(Json(FeatureListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Get feature details
#[utoipa::path(
    get,
    tag = "Features",
    path = "/api/v1/features/{feature_id}",
    params(("feature_id" = FeatureId, Path, description = "Feature ID")),
    responses(
        (status = 200, description = "Feature details", body = Feature),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(feature_id): Path<FeatureId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let feature = app_state
        .store
        .get_feature(feature_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching feature: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::feature_to_rest(feature)))
}

/// Verify that `entity` belongs to `tenant_id`. Returns a 404 if not found.
async fn verify_entity_ownership(
    app_state: &AppState,
    tenant_id: common_domain::ids::TenantId,
    entity: &EntitlementEntityId,
) -> Result<(), RestApiError> {
    match entity {
        EntitlementEntityId::Plan(id) => {
            app_state
                .store
                .get_plan_overview(*id, tenant_id)
                .await
                .map_err(RestApiError::from)?;
        }
        EntitlementEntityId::PlanVersion(id) => {
            app_state
                .store
                .get_plan_version_by_id(*id, tenant_id)
                .await
                .map_err(RestApiError::from)?;
        }
        EntitlementEntityId::AddOn(id) => {
            app_state
                .store
                .get_add_on_by_id(tenant_id, *id)
                .await
                .map_err(RestApiError::from)?;
        }
        EntitlementEntityId::Subscription(id) => {
            app_state
                .store
                .get_subscription(tenant_id, *id)
                .await
                .map_err(RestApiError::from)?;
        }
        EntitlementEntityId::Feature(id) => {
            app_state
                .store
                .get_feature(*id, tenant_id)
                .await
                .map_err(RestApiError::from)?;
        }
        EntitlementEntityId::Quote(id) => {
            app_state
                .store
                .get_quote_by_id(tenant_id, *id)
                .await
                .map_err(RestApiError::from)?;
        }
    }
    Ok(())
}

// ── Entity entitlement list endpoints ─────────────────────────

async fn list_entitlements_for(
    app_state: AppState,
    tenant_id: common_domain::ids::TenantId,
    entity: EntitlementEntityId,
) -> Result<impl IntoResponse, RestApiError> {
    verify_entity_ownership(&app_state, tenant_id, &entity).await?;

    let entitlements = app_state
        .store
        .list_entitlements_by_entity(entity, tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error listing entitlements: {e}");
            RestApiError::StoreError
        })?;

    let data = entitlements
        .into_iter()
        .map(mapping::entitlement_to_rest)
        .collect();
    Ok(Json(EntitlementListResponse { data }))
}

/// List plan version entitlements
#[utoipa::path(
    get,
    tags = ["Plans", "Entitlements"],
    path = "/api/v1/plan-versions/{plan_version_id}/entitlements",
    params(("plan_version_id" = PlanVersionId, Path, description = "Plan version ID")),
    responses(
        (status = 200, description = "Entitlements for this plan version", body = EntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan version not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_plan_version_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_version_id): Path<PlanVersionId>,
) -> Result<impl IntoResponse, RestApiError> {
    list_entitlements_for(
        app_state,
        authorized_state.tenant_id,
        EntitlementEntityId::PlanVersion(plan_version_id),
    )
    .await
}

/// List add-on entitlements
#[utoipa::path(
    get,
    tags = ["Add-ons", "Entitlements"],
    path = "/api/v1/addons/{addon_id}/entitlements",
    params(("addon_id" = AddOnId, Path, description = "Add-on ID")),
    responses(
        (status = 200, description = "Entitlements for this add-on", body = EntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Add-on not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_add_on_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(add_on_id): Path<AddOnId>,
) -> Result<impl IntoResponse, RestApiError> {
    list_entitlements_for(
        app_state,
        authorized_state.tenant_id,
        EntitlementEntityId::AddOn(add_on_id),
    )
    .await
}

/// List subscription entitlements
#[utoipa::path(
    get,
    tags = ["Subscriptions", "Entitlements"],
    path = "/api/v1/subscriptions/{subscription_id}/entitlements",
    params(("subscription_id" = SubscriptionId, Path, description = "Subscription ID")),
    responses(
        (status = 200, description = "Entitlements for this subscription", body = EntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_subscription_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    list_entitlements_for(
        app_state,
        authorized_state.tenant_id,
        EntitlementEntityId::Subscription(subscription_id),
    )
    .await
}

// ── Customer resolution ────────────────────────────────────────

/// List customer entitlements
#[utoipa::path(
    get,
    tags = ["Entitlements", "Customers"],
    path = "/api/v1/customers/{id_or_alias}/entitlements",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
    ),
    responses(
        (status = 200, description = "Customer entitlements", body = EffectiveEntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_effective_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(Path(id_or_alias)): Valid<Path<AliasOr<CustomerId>>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;

    let resolved = app_state
        .services
        .get_effective_entitlements(customer.id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving customer entitlements: {e}");
            RestApiError::from(e)
        })?;

    let data = resolved
        .into_iter()
        .map(mapping::effective_entitlement_to_rest)
        .collect();
    Ok(Json(EffectiveEntitlementListResponse { data }))
}

// ── Product entitlements ───────────────────────────────────────

/// List product entitlements
#[utoipa::path(
    get,
    tags = ["Products", "Entitlements"],
    path = "/api/v1/products/{product_id}/entitlements",
    params(("product_id" = ProductId, Path, description = "Product ID")),
    responses(
        (status = 200, description = "List entitlements for this product", body = ResolvedEntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Product not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_product_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(product_id): Path<ProductId>,
) -> Result<impl IntoResponse, RestApiError> {
    let mut conn = app_state.store.get_conn().await.map_err(|e| {
        log::error!("Error getting db connection: {e}");
        RestApiError::StoreError
    })?;
    let resolved = app_state
        .store
        .resolve_for_entity(
            &mut conn,
            authorized_state.tenant_id,
            ResolveTarget::Product(product_id),
        )
        .await
        .map_err(|e| {
            log::error!("Error resolving product entitlements: {e}");
            RestApiError::from(e)
        })?;
    Ok(Json(ResolvedEntitlementListResponse {
        data: resolved
            .into_iter()
            .map(mapping::resolved_entitlement_to_rest)
            .collect(),
    }))
}
