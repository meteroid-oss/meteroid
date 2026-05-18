use super::AppState;

use crate::api_rest::entitlements::mapping::feature_entitlement_spec_from_rest;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{
    AddOnId, AliasOr, CustomerId, EntitlementEntityId, EntitlementId, FeatureId, PlanId,
    PlanVersionId, QuoteId, SubscriptionId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::entitlements::{
    EntitlementNew, EntitlementUpdate, FeatureNew, FeatureUpdate,
};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::repositories::plans::PlansInterface;
use meteroid_store::repositories::quotes::QuotesInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterface;

use crate::api_rest::QueryParams;
use crate::api_rest::entitlements::mapping;
use crate::api_rest::entitlements::model::{
    CreateFeatureRequest, EffectiveEntitlement, EffectiveEntitlementListResponse, Entitlement,
    EntitlementListResponse, EntitlementSpec, Feature, FeatureListRequest, FeatureListResponse,
    SetFeatureStatusRequest, UpdateEntitlementRequest, UpdateFeatureRequest,
};
use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;

/// List features
#[utoipa::path(
    get,
    tag = "Entitlements",
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
    tag = "Entitlements",
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

/// Create a feature
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/features",
    request_body(content = CreateFeatureRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Feature created", body = Feature),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreateFeatureRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let entitlement = payload.entitlement.map(feature_entitlement_spec_from_rest);

    let feature = app_state
        .store
        .create_feature(FeatureNew {
            tenant_id: authorized_state.tenant_id,
            product_id: payload.product_id,
            name: payload.name,
            description: payload.description,
            feature_type: payload.feature_type.into(),
            created_by: authorized_state.actor_id,
            entitlement,
        })
        .await
        .map_err(|e| {
            log::error!("Error creating feature: {e}");
            RestApiError::from(e)
        })?;

    Ok((StatusCode::CREATED, Json(mapping::feature_to_rest(feature))))
}

/// Update a feature
///
/// Partially update feature fields. Feature type is immutable.
#[utoipa::path(
    patch,
    tag = "Entitlements",
    path = "/api/v1/features/{feature_id}",
    params(("feature_id" = FeatureId, Path, description = "Feature ID")),
    request_body(content = UpdateFeatureRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Feature updated", body = Feature),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn update_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(feature_id): Path<FeatureId>,
    Valid(Json(payload)): Valid<Json<UpdateFeatureRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let feature = app_state
        .store
        .update_feature(
            feature_id,
            authorized_state.tenant_id,
            FeatureUpdate {
                name: payload.name,
                description: payload.description,
                product_id: payload.product_id,
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error updating feature: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::feature_to_rest(feature)))
}

/// Set feature status (Active / Disabled / Archived)
///
/// `Disabled` is the operator kill switch — the feature is hidden from resolution while
/// its entitlements stay intact. `Archived` is the long-form retire action. `Active` restores
/// the feature.
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/features/{feature_id}/status",
    params(("feature_id" = FeatureId, Path, description = "Feature ID")),
    request_body(content = SetFeatureStatusRequest, content_type = "application/json"),
    responses(
        (status = 204, description = "Feature status updated"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn set_feature_status(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(feature_id): Path<FeatureId>,
    Valid(Json(payload)): Valid<Json<SetFeatureStatusRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .set_feature_status(
            feature_id,
            authorized_state.tenant_id,
            payload.status.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error updating feature status: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

/// List entitlements that target a feature across all entities
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/v1/features/{feature_id}/entitlements",
    params(("feature_id" = FeatureId, Path, description = "Feature ID")),
    responses(
        (status = 200, description = "Entitlements that target this feature", body = EntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_entitlements_by_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(feature_id): Path<FeatureId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    // 404-on-foreign-tenant probe protection: load the feature first.
    app_state
        .store
        .get_feature(feature_id, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;

    let entitlements = app_state
        .store
        .list_entitlements_by_feature(feature_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error listing entitlements by feature: {e}");
            RestApiError::StoreError
        })?;

    let data = entitlements
        .into_iter()
        .map(mapping::entitlement_to_rest)
        .collect();
    Ok(Json(EntitlementListResponse { data }))
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

async fn create_entitlement_for(
    app_state: AppState,
    tenant_id: common_domain::ids::TenantId,
    actor_id: uuid::Uuid,
    entity: EntitlementEntityId,
    payload: EntitlementSpec,
) -> Result<impl IntoResponse, RestApiError> {
    verify_entity_ownership(&app_state, tenant_id, &entity).await?;

    let entitlement = app_state
        .store
        .create_entitlement(EntitlementNew {
            tenant_id,
            feature_id: payload.feature_id,
            entity,
            value: mapping::value_from_rest(payload.value),
            created_by: actor_id,
        })
        .await
        .map_err(|e| {
            log::error!("Error creating entitlement: {e}");
            RestApiError::from(e)
        })?;

    Ok((
        StatusCode::CREATED,
        Json(mapping::entitlement_to_rest(entitlement)),
    ))
}

/// List entitlements on a plan
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/v1/plans/{plan_id}/entitlements",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    responses(
        (status = 200, description = "Entitlements for this plan", body = EntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_plan_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(plan_id): Path<PlanId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    list_entitlements_for(
        app_state,
        authorized_state.tenant_id,
        EntitlementEntityId::Plan(plan_id),
    )
    .await
}

/// Add an entitlement to a plan
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/plans/{plan_id}/entitlements",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    request_body(content = EntitlementSpec, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlement created", body = Entitlement),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_plan_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
    Valid(Json(payload)): Valid<Json<EntitlementSpec>>,
) -> Result<impl IntoResponse, RestApiError> {
    create_entitlement_for(
        app_state,
        authorized_state.tenant_id,
        authorized_state.actor_id,
        EntitlementEntityId::Plan(plan_id),
        payload,
    )
    .await
}

/// Add a feature-level (baseline) entitlement
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/features/{feature_id}/entitlement",
    params(("feature_id" = FeatureId, Path, description = "Feature ID")),
    request_body(content = EntitlementSpec, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlement created", body = Entitlement),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_feature_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(feature_id): Path<FeatureId>,
    Valid(Json(payload)): Valid<Json<EntitlementSpec>>,
) -> Result<impl IntoResponse, RestApiError> {
    create_entitlement_for(
        app_state,
        authorized_state.tenant_id,
        authorized_state.actor_id,
        EntitlementEntityId::Feature(feature_id),
        payload,
    )
    .await
}

// ── Plan version entitlements ──────────────────────────────────

/// Add an entitlement to a plan version
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/plan-versions/{plan_version_id}/entitlements",
    params(("plan_version_id" = PlanVersionId, Path, description = "Plan version ID")),
    request_body(content = EntitlementSpec, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlement created", body = Entitlement),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_plan_version_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_version_id): Path<PlanVersionId>,
    Valid(Json(payload)): Valid<Json<EntitlementSpec>>,
) -> Result<impl IntoResponse, RestApiError> {
    create_entitlement_for(
        app_state,
        authorized_state.tenant_id,
        authorized_state.actor_id,
        EntitlementEntityId::PlanVersion(plan_version_id),
        payload,
    )
    .await
}

// ── Add-on entitlements ────────────────────────────────────────

/// Add an entitlement to an add-on
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/addons/{addon_id}/entitlements",
    params(("addon_id" = AddOnId, Path, description = "Add-on ID")),
    request_body(content = EntitlementSpec, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlement created", body = Entitlement),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_add_on_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(add_on_id): Path<AddOnId>,
    Valid(Json(payload)): Valid<Json<EntitlementSpec>>,
) -> Result<impl IntoResponse, RestApiError> {
    create_entitlement_for(
        app_state,
        authorized_state.tenant_id,
        authorized_state.actor_id,
        EntitlementEntityId::AddOn(add_on_id),
        payload,
    )
    .await
}

// ── Subscription entitlements ──────────────────────────────────

/// Add an entitlement to a subscription
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/subscriptions/{subscription_id}/entitlements",
    params(("subscription_id" = SubscriptionId, Path, description = "Subscription ID")),
    request_body(content = EntitlementSpec, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlement created", body = Entitlement),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_subscription_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(subscription_id): Path<SubscriptionId>,
    Valid(Json(payload)): Valid<Json<EntitlementSpec>>,
) -> Result<impl IntoResponse, RestApiError> {
    create_entitlement_for(
        app_state,
        authorized_state.tenant_id,
        authorized_state.actor_id,
        EntitlementEntityId::Subscription(subscription_id),
        payload,
    )
    .await
}

// ── Quote entitlements ─────────────────────────────────────────

/// Add an entitlement to a quote
#[utoipa::path(
    post,
    tag = "Entitlements",
    path = "/api/v1/quotes/{quote_id}/entitlements",
    params(("quote_id" = QuoteId, Path, description = "Quote ID")),
    request_body(content = EntitlementSpec, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlement created", body = Entitlement),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Quote not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_quote_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(quote_id): Path<QuoteId>,
    Valid(Json(payload)): Valid<Json<EntitlementSpec>>,
) -> Result<impl IntoResponse, RestApiError> {
    create_entitlement_for(
        app_state,
        authorized_state.tenant_id,
        authorized_state.actor_id,
        EntitlementEntityId::Quote(quote_id),
        payload,
    )
    .await
}

// ── Flat entitlement operations (update / delete) ──────────────

/// Update an entitlement
#[utoipa::path(
    patch,
    tag = "Entitlements",
    path = "/api/v1/entitlements/{entitlement_id}",
    params(("entitlement_id" = EntitlementId, Path, description = "Entitlement ID")),
    request_body(content = UpdateEntitlementRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Entitlement updated", body = Entitlement),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Entitlement not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn update_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(entitlement_id): Path<EntitlementId>,
    Valid(Json(payload)): Valid<Json<UpdateEntitlementRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let value = payload.value.map(mapping::value_from_rest);

    let entitlement = app_state
        .store
        .update_entitlement(
            entitlement_id,
            authorized_state.tenant_id,
            EntitlementUpdate { value },
        )
        .await
        .map_err(|e| {
            log::error!("Error updating entitlement: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::entitlement_to_rest(entitlement)))
}

/// Delete an entitlement
#[utoipa::path(
    delete,
    tag = "Entitlements",
    path = "/api/v1/entitlements/{entitlement_id}",
    params(("entitlement_id" = EntitlementId, Path, description = "Entitlement ID")),
    responses(
        (status = 204, description = "Entitlement deleted"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Entitlement not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn delete_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(entitlement_id): Path<EntitlementId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .delete_entitlement(entitlement_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error deleting entitlement: {e}");
            RestApiError::from(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

// ── Customer resolution ────────────────────────────────────────

/// Get all entitlements for a customer
#[utoipa::path(
    get,
    tag = "Entitlements",
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

/// Get a single entitlement for a customer
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/v1/customers/{id_or_alias}/entitlements/{feature_id}",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("feature_id" = FeatureId, Path, description = "Feature ID"),
    ),
    responses(
        (status = 200, description = "Customer entitlement for this feature", body = EffectiveEntitlement),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_effective_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path((id_or_alias, feature_id)): Path<(AliasOr<CustomerId>, FeatureId)>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;

    let entitlement = app_state
        .services
        .get_effective_entitlement_for_feature(customer.id, authorized_state.tenant_id, feature_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving customer entitlement: {e}");
            RestApiError::from(e)
        })?
        .ok_or(RestApiError::NotFound)?;

    Ok(Json(mapping::effective_entitlement_to_rest(entitlement)))
}
