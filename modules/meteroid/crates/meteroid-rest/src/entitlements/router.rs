use super::AppState;
use meteroid_rest_common::extract::{ApiJson, ApiPath, ApiValid};

use axum::extract::{RawQuery, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, AliasOr, ApiTokenId, CustomerId, EntitlementEntityId, EntitlementId, FeatureId,
    PlanVersionId, ProductId, SubscriptionId, TenantId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use meteroid_store::domain::entitlement_checks::{
    CheckRequest, SnapshotRead, effective_usage_warnings,
};
use meteroid_store::domain::entitlement_fold::{Effect, EvalLevel, Tier};
use meteroid_store::domain::entitlements::{
    EntitlementSpec, Feature as DomainFeature, FeatureKind as DomainFeatureKind,
    LimitCountedBy as DomainCountedBy,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::resources::ResourceInput;
use meteroid_store::domain::{FeatureListOrder, FeatureNew, FeatureUpdate};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::entitlements::{EntitlementsInterface, ResolveTarget};
use meteroid_store::repositories::plans::PlansInterface;
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::repositories::quotes::QuotesInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterface;
use meteroid_store::services::entitlements::app_counts::{AppCountOp, AppCountWrite};

use crate::entitlements::mapping;
use crate::entitlements::model::{
    CreateCustomerEntitlementsRequest, CreateEntitlementsRequest, CreateFeatureRequest,
    CustomerFeaturePath, EffectiveEntitlementListResponse, Entitlement, EntitlementCheck,
    EntitlementCheckQuery, EntitlementCheckRequest, EntitlementHistoryResponse,
    EntitlementListResponse, EntitlementSnapshot, EntitlementSpecRequest, EntitlementValue,
    Feature, FeatureListRequest, FeatureListResponse, ReportUsageRequest, ReportUsageResponse,
    ResolvedEntitlementListResponse, UpdateEntitlementRequest, UpdateFeatureRequest,
};
use crate::resources::error::ApiError;
use meteroid_app::errors::RestApiError;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::PaginationExt;

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
pub async fn list_features(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    ApiValid(QueryParams(request)): ApiValid<QueryParams<FeatureListRequest>>,
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
            None,
            FeatureListOrder::default(),
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
    path = "/api/v1/features/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Feature ID or code")),
    responses(
        (status = 200, description = "Feature details", body = Feature),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn get_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    ApiValid(ApiPath(id_or_code)): ApiValid<ApiPath<AliasOr<FeatureId>>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let feature = app_state
        .store
        .get_feature_by_id_or_code(id_or_code, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching feature: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::feature_to_rest(feature)))
}

// ── Create feature ─────────────────────────────────────────────

/// Create a feature
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v1/features",
    request_body(content = CreateFeatureRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Feature created", body = Feature),
        (status = 400, description = "Bad request, including an unknown product or metric reference", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 409, description = "A feature with this code already exists", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn create_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<CreateFeatureRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = authorized_state.tenant_id;

    let v1_defaults = payload.feature_type.is_some()
        && matches!(payload.entitlement, Some(EntitlementValue::Metered(_)));
    let kind = mapping::created_feature_kind(
        payload.kind,
        payload.value,
        payload.limit,
        payload.feature_type,
    )
    .map_err(RestApiError::InvalidInput)?;
    let terms = payload
        .entitlement
        .as_ref()
        .map(mapping::legacy_terms)
        .transpose()
        .map_err(RestApiError::InvalidInput)?
        .flatten();
    let policy =
        mapping::created_feature_policy(&kind, payload.policy.map(Into::into), terms, v1_defaults)
            .map_err(RestApiError::InvalidInput)?;

    // Both are body references, so a miss is a 400 rather than the path resource's 404.
    if let Some(product_id) = payload.product_id {
        app_state
            .store
            .find_product_by_id(product_id, tenant_id)
            .await
            .map_err(|e| body_reference_error(e, format!("product {product_id} not found")))?;
    }

    if let DomainFeatureKind::Limit(DomainCountedBy::Events(metric_id)) = &kind {
        let metric_id = *metric_id;
        app_state
            .store
            .find_billable_metric_by_id(metric_id, tenant_id)
            .await
            .map_err(|e| body_reference_error(e, format!("metric {metric_id} not found")))?;
    }

    let feature = app_state
        .store
        .create_feature(
            authorized_state.as_actor(),
            FeatureNew {
                tenant_id,
                product_id: payload.product_id,
                name: payload.name,
                code: payload.code,
                description: payload.description,
                kind,
                unit: payload.unit,
                level_type_id: payload.level,
                policy,
                default: payload.entitlement.map(mapping::value_to_domain),
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error creating feature: {e}");
            ApiError::from(e)
        })?;

    Ok((StatusCode::CREATED, Json(mapping::feature_to_rest(feature))))
}

// ── Update feature ─────────────────────────────────────────────

/// Update a feature
///
/// Partially update feature fields. Code, kind and product are immutable. Changing the level moves
/// the rows on the feature's level with it.
#[utoipa::path(
    patch,
    tag = "Features",
    path = "/api/v1/features/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Feature ID or code")),
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
pub async fn update_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiPath(id_or_code)): ApiValid<ApiPath<AliasOr<FeatureId>>>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<UpdateFeatureRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let feature = app_state
        .store
        .get_feature_by_id_or_code(id_or_code, authorized_state.tenant_id)
        .await
        .map_err(ApiError::from)?;

    let updated = app_state
        .store
        .update_feature(
            feature.id,
            authorized_state.tenant_id,
            FeatureUpdate {
                name: payload.name,
                description: payload.description,
                product_id: None,
                unit: payload.unit,
                number_higher_is_better: payload.higher_is_better,
                select_options: payload.select_options,
                select_ordered: payload.select_ordered,
                level_type_id: payload.level,
                policy: payload.policy.map(Into::into),
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error updating feature: {e}");
            ApiError::from(e)
        })?;

    Ok(Json(mapping::feature_to_rest(updated)))
}

// ── Archive / Unarchive feature ────────────────────────────────

fn feature_status_error(e: error_stack::Report<StoreError>) -> RestApiError {
    match e.current_context() {
        StoreError::FailedPrecondition(msg) => RestApiError::FailedPrecondition(msg.clone()),
        _ => {
            log::error!("Error setting feature status: {e}");
            RestApiError::from(e)
        }
    }
}

async fn transition_feature_status(
    app_state: AppState,
    authorized_state: AuthorizedAsTenant,
    id: FeatureId,
    status: FeatureStatusEnum,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .set_feature_status(
            authorized_state.as_actor(),
            id,
            authorized_state.tenant_id,
            status,
        )
        .await
        .map_err(feature_status_error)
        .map(|_| StatusCode::NO_CONTENT)
}

/// Archive a feature
///
/// Turns the feature off and hides it and its entitlements until it is unarchived, and frees its
/// code for a new feature.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v1/features/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Feature ID or code")),
    responses(
        (status = 204, description = "Feature archived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn archive_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiPath(id_or_code)): ApiValid<ApiPath<AliasOr<FeatureId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    let feature = app_state
        .store
        .get_feature_by_id_or_code(id_or_code, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;
    transition_feature_status(
        app_state,
        authorized_state,
        feature.id,
        FeatureStatusEnum::Archived,
    )
    .await
}

/// Unarchive a feature
///
/// Makes the feature active again with all its entitlements. Fails while another feature uses its
/// code.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v1/features/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Feature ID or code")),
    responses(
        (status = 204, description = "Feature unarchived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Feature not found", body = RestErrorResponse),
        (status = 409, description = "Feature code conflict", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn unarchive_feature(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiPath(id_or_code)): ApiValid<ApiPath<AliasOr<FeatureId>>>,
) -> Result<impl IntoResponse, RestApiError> {
    let id = match id_or_code {
        AliasOr::Id(id) => id,
        AliasOr::Alias(code) => app_state
            .store
            .find_archived_feature_by_code(&code, authorized_state.tenant_id)
            .await
            .map_err(feature_status_error)?,
    };
    transition_feature_status(app_state, authorized_state, id, FeatureStatusEnum::Active).await
}

/// A reference named in the request body is the caller's input, so a miss is a 400 rather
/// than the 404 reserved for the path resource.
fn body_reference_error(err: error_stack::Report<StoreError>, message: String) -> RestApiError {
    match err.current_context() {
        StoreError::ValueNotFound(_) => RestApiError::InvalidInput(message),
        _ => RestApiError::from(err),
    }
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
        EntitlementEntityId::Customer(id) => {
            app_state
                .store
                .find_customer_by_id_or_alias(AliasOr::Id(*id), tenant_id)
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

const TERMS_AND_OVERRIDE: &str = "`policy_override` and the deprecated limit terms of `value` \
                                  disagree: send `policy_override` alone";

/// A row of `feature` to create, with its deprecated limit terms made into its `policy_override`.
fn spec_to_domain(
    spec: EntitlementSpecRequest,
    tier: Tier,
    feature: &DomainFeature,
) -> Result<EntitlementSpec, RestApiError> {
    let terms = mapping::legacy_terms(&spec.value).map_err(RestApiError::InvalidInput)?;
    let mut spec = mapping::entitlement_spec_to_domain(spec).map_err(RestApiError::InvalidInput)?;
    let Some(terms) = terms else {
        return Ok(spec);
    };
    // `false`: an add-on defaults to Add or Raise to either way, never to Set.
    let is_set = spec
        .effect
        .unwrap_or_else(|| tier.default_effect(&feature.value_order(), false))
        == Effect::Set;
    let in_effect = spec
        .policy_override
        .clone()
        .unwrap_or_else(|| feature.policy_or_default());
    if let Some(policy_override) =
        mapping::override_for_terms(feature, &in_effect, &terms, is_set, &spec.value)
            .map_err(RestApiError::InvalidInput)?
    {
        if spec.policy_override.is_some() {
            return Err(RestApiError::InvalidInput(TERMS_AND_OVERRIDE.into()));
        }
        spec.policy_override = policy_override;
    }
    Ok(spec)
}

/// The rows of `tier` to create; reads the feature of each one carrying deprecated limit terms.
pub(crate) async fn specs_to_domain(
    app_state: &AppState,
    tenant_id: TenantId,
    tier: Tier,
    specs: Vec<EntitlementSpecRequest>,
) -> Result<Vec<EntitlementSpec>, RestApiError> {
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        let carries_terms = mapping::legacy_terms(&spec.value)
            .map_err(RestApiError::InvalidInput)?
            .is_some();
        if !carries_terms {
            out.push(
                mapping::entitlement_spec_to_domain(spec).map_err(RestApiError::InvalidInput)?,
            );
            continue;
        }
        let feature_id = spec.feature_id;
        let feature = app_state
            .store
            .get_feature(feature_id, tenant_id)
            .await
            .map_err(|e| body_reference_error(e, format!("feature {feature_id} not found")))?;
        out.push(spec_to_domain(spec, tier, &feature)?);
    }
    Ok(out)
}

// ── Entity entitlement list endpoints ─────────────────────────

async fn resolve_entitlements_for(
    app_state: AppState,
    tenant_id: common_domain::ids::TenantId,
    target: ResolveTarget,
    entity: EntitlementEntityId,
) -> Result<impl IntoResponse, RestApiError> {
    verify_entity_ownership(&app_state, tenant_id, &entity).await?;

    let mut conn = app_state.store.get_conn().await.map_err(|e| {
        log::error!("Error getting db connection: {e}");
        RestApiError::StoreError
    })?;
    let resolved = app_state
        .store
        .resolve_entitlements_for_entity(&mut conn, tenant_id, target)
        .await
        .map_err(|e| {
            log::error!("Error resolving entitlements: {e}");
            RestApiError::from(e)
        })?;

    let data = resolved
        .into_iter()
        .map(mapping::resolved_entitlement_to_rest)
        .collect();
    Ok(Json(ResolvedEntitlementListResponse { data }))
}

async fn batch_create_entitlements_for(
    app_state: AppState,
    authorized_state: AuthorizedAsTenant,
    entity: EntitlementEntityId,
    request: CreateEntitlementsRequest,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = authorized_state.tenant_id;
    verify_entity_ownership(&app_state, tenant_id, &entity).await?;

    let specs = specs_to_domain(
        &app_state,
        tenant_id,
        Tier::of(&entity, false),
        request.entitlements,
    )
    .await?;

    let created = app_state
        .store
        .batch_create_entitlements(&authorized_state.as_actor(), tenant_id, entity, specs)
        .await
        .map_err(|e| {
            log::error!("Error creating entitlements: {e}");
            ApiError::from(e)
        })?;

    let data = created
        .into_iter()
        .map(mapping::entitlement_to_rest)
        .collect();

    Ok((StatusCode::CREATED, Json(EntitlementListResponse { data })))
}

/// List plan version entitlements
#[utoipa::path(
    get,
    tags = ["Plans", "Entitlements"],
    path = "/api/v1/plan-versions/{plan_version_id}/entitlements",
    params(("plan_version_id" = PlanVersionId, Path, description = "Plan version ID")),
    responses(
        (status = 200, description = "Resolved entitlements for this plan version", body = ResolvedEntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan version not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn list_plan_version_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(plan_version_id): ApiPath<PlanVersionId>,
) -> Result<impl IntoResponse, RestApiError> {
    resolve_entitlements_for(
        app_state,
        authorized_state.tenant_id,
        ResolveTarget::PlanVersion(plan_version_id),
        EntitlementEntityId::PlanVersion(plan_version_id),
    )
    .await
}

/// Create plan version entitlements
///
/// Entitlements already present on this plan version are skipped.
#[utoipa::path(
    post,
    tags = ["Plans", "Entitlements"],
    path = "/api/v1/plan-versions/{plan_version_id}/entitlements",
    params(("plan_version_id" = PlanVersionId, Path, description = "Plan version ID")),
    request_body(content = CreateEntitlementsRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlements created", body = EntitlementListResponse),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan version not found", body = RestErrorResponse),
        (status = 409, description = "The plan version is published: write them through a draft", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn create_plan_version_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(plan_version_id): ApiPath<PlanVersionId>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<CreateEntitlementsRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    batch_create_entitlements_for(
        app_state,
        authorized_state,
        EntitlementEntityId::PlanVersion(plan_version_id),
        payload,
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
        (status = 200, description = "Resolved entitlements for this add-on", body = ResolvedEntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Add-on not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn list_add_on_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(add_on_id): ApiPath<AddOnId>,
) -> Result<impl IntoResponse, RestApiError> {
    resolve_entitlements_for(
        app_state,
        authorized_state.tenant_id,
        ResolveTarget::AddOn(add_on_id),
        EntitlementEntityId::AddOn(add_on_id),
    )
    .await
}

/// Create add-on entitlements
///
/// Entitlements already present on this add-on are skipped.
#[utoipa::path(
    post,
    tags = ["Add-ons", "Entitlements"],
    path = "/api/v1/addons/{addon_id}/entitlements",
    params(("addon_id" = AddOnId, Path, description = "Add-on ID")),
    request_body(content = CreateEntitlementsRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlements created", body = EntitlementListResponse),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Add-on not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn create_add_on_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(add_on_id): ApiPath<AddOnId>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<CreateEntitlementsRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    batch_create_entitlements_for(
        app_state,
        authorized_state,
        EntitlementEntityId::AddOn(add_on_id),
        payload,
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
        (status = 200, description = "Effective entitlements for this subscription, with live usage", body = EffectiveEntitlementListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Subscription not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn list_subscription_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(subscription_id): ApiPath<SubscriptionId>,
) -> Result<impl IntoResponse, RestApiError> {
    let resolved = app_state
        .services
        .get_effective_entitlements_for_subscription(subscription_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving subscription entitlements: {e}");
            RestApiError::from(e)
        })?;

    let warnings = effective_usage_warnings(&resolved);
    let data = resolved
        .into_iter()
        .map(mapping::effective_entitlement_to_rest)
        .collect();
    Ok(Json(EffectiveEntitlementListResponse { data, warnings }))
}

// ── Customer resolution ────────────────────────────────────────

/// List customer entitlements
///
/// Every feature with its levels: the customer, and the resource named by `resources`, each with
/// its value, usage and policy. The `ETag` header changes with the values, not the usage: send it
/// back as `If-None-Match` to get a 304 while nothing changed. A level whose usage cannot be read
/// has no `used`, and the warning `usage_unavailable`.
#[utoipa::path(
    get,
    tags = ["Customers", "Entitlements"],
    path = "/api/v1/customers/{id_or_alias}/entitlements",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("resources" = Option<std::collections::HashMap<String, String>>, Query, style = DeepObject, explode, description = "At most one resource, as `resources[<key>]=<value>`"),
        ("If-None-Match" = Option<String>, Header, description = "The `ETag` of a previous answer"),
    ),
    responses(
        (status = 200, description = "Customer entitlements", body = EntitlementSnapshot),
        (status = 304, description = "Unchanged since the `If-None-Match` etag"),
        (status = 400, description = "Unknown resource key, or more than one resource", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn get_entitlement_snapshot(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    ApiValid(ApiPath(id_or_alias)): ApiValid<ApiPath<AliasOr<CustomerId>>>,
    State(app_state): State<AppState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;
    snapshot_response(
        &app_state,
        authorized_state.tenant_id,
        customer.id,
        resource_input(query.as_deref()),
        &headers,
    )
    .await
}

pub(crate) fn resource_input(query: Option<&str>) -> ResourceInput {
    ResourceInput {
        resources: mapping::bracketed_params(query, "resources"),
        attributes: mapping::bracketed_params(query, "attributes"),
    }
}

fn etag_header(etag: &str) -> Result<HeaderValue, RestApiError> {
    HeaderValue::from_str(&format!("\"{etag}\"")).map_err(|_| RestApiError::StoreError)
}

/// The codes of the types among `levels`, read only when one is on a type.
async fn type_codes(
    app_state: &AppState,
    tenant_id: TenantId,
    mut levels: impl Iterator<Item = EvalLevel>,
) -> Result<mapping::TypeCodes, RestApiError> {
    if !levels.any(|l| l.type_id().is_some()) {
        return Ok(Default::default());
    }
    let types = app_state
        .store
        .list_resource_types(tenant_id, true)
        .await
        .map_err(RestApiError::from)?;
    Ok(types.into_iter().map(|t| (t.id, t.code)).collect())
}

/// The store matches `If-None-Match`: weak or strong tags, lists, `*`.
pub(crate) async fn snapshot_response(
    app_state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    resources: ResourceInput,
    headers: &HeaderMap,
) -> Result<Response, RestApiError> {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    let read = app_state
        .store
        .entitlement_snapshot(tenant_id, customer_id, &resources, if_none_match)
        .await
        .map_err(RestApiError::from)?;
    Ok(match read {
        SnapshotRead::NotModified { etag } => (
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, etag_header(&etag)?)],
        )
            .into_response(),
        SnapshotRead::Modified(snapshot) => {
            let levels = snapshot.features.iter().flat_map(|f| &f.levels);
            let codes = type_codes(app_state, tenant_id, levels.map(|l| l.level)).await?;
            (
                [(header::ETAG, etag_header(&snapshot.etag)?)],
                Json(mapping::snapshot_to_rest(snapshot, &codes)),
            )
                .into_response()
        }
    })
}

/// Create customer overrides
///
/// Rows on the customer, or with `resource_id` on one of its stored resources. `effect` defaults
/// to Raise to. All the rows are written, or none.
#[utoipa::path(
    post,
    tags = ["Customers", "Entitlements"],
    path = "/api/v1/customers/{id_or_alias}/entitlements",
    params(("id_or_alias" = String, Path, description = "Customer ID or alias")),
    request_body(content = CreateCustomerEntitlementsRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlements created", body = EntitlementListResponse),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer not found", body = RestErrorResponse),
        (status = 409, description = "A row with the same feature, level, resource and condition exists", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn create_customer_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiPath(id_or_alias)): ApiValid<ApiPath<AliasOr<CustomerId>>>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<CreateCustomerEntitlementsRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = authorized_state.tenant_id;
    let actor = authorized_state.as_actor();
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias, tenant_id)
        .await
        .map_err(ApiError::from)?;
    let entity = EntitlementEntityId::Customer(customer.id);
    let specs = specs_to_domain(
        &app_state,
        tenant_id,
        Tier::of(&entity, payload.resource_id.is_some()),
        payload.entitlements,
    )
    .await?;
    let data = app_state
        .store
        .create_entitlements(&actor, tenant_id, entity, payload.resource_id, specs)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(mapping::entitlement_to_rest)
        .collect();
    Ok((StatusCode::CREATED, Json(EntitlementListResponse { data })))
}

// ── Check ──────────────────────────────────────────────────────

/// The key a server call came with, for the asserted-attribute log line.
fn api_key_of(auth: &AuthorizedAsTenant) -> Option<ApiTokenId> {
    match auth.as_actor() {
        Actor::ApiToken { id } => Some(id),
        _ => None,
    }
}

const DEPRECATION: HeaderName = HeaderName::from_static("deprecation");

/// Answers the check; a `subscription_id`, ignored, adds a `Deprecation` header.
pub(crate) async fn check_response(
    app_state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    request: EntitlementCheckRequest,
    api_key_id: Option<ApiTokenId>,
) -> Result<Response, RestApiError> {
    if let Some(dropped) =
        EntitlementCheckRequest::dropped_input(request.other.keys().map(String::as_str))
    {
        return Err(RestApiError::InvalidInput(format!(
            "`{dropped}` is not supported by the check"
        )));
    }
    let deprecated = request.subscription_id.is_some();
    let check = CheckRequest {
        resources: request.resources,
        attributes: request.attributes,
        requested: request.requested,
        current_count: request.current_count,
        explain: request.explain,
        subscription_id: request.subscription_id,
        api_key_id,
    };
    let answer = app_state
        .store
        .check_entitlement(tenant_id, customer_id, request.feature, check)
        .await
        .map_err(RestApiError::from)?;
    let codes = type_codes(app_state, tenant_id, answer.levels.iter().map(|l| l.level)).await?;
    let body = Json(mapping::check_to_rest(answer, &codes));
    Ok(if deprecated {
        ([(DEPRECATION, HeaderValue::from_static("true"))], body).into_response()
    } else {
        body.into_response()
    })
}

/// The `GET` alias's inputs, as the `POST` body carries them.
pub(crate) fn check_request_of(
    feature: AliasOr<FeatureId>,
    query: EntitlementCheckQuery,
    raw: Option<&str>,
) -> EntitlementCheckRequest {
    let ResourceInput {
        resources,
        attributes,
    } = resource_input(raw);
    let other = url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes())
        .filter(|(key, _)| {
            EntitlementCheckRequest::dropped_input(std::iter::once(key.as_ref())).is_some()
        })
        .map(|(key, value)| {
            (
                key.into_owned(),
                serde_json::Value::String(value.into_owned()),
            )
        })
        .collect();
    EntitlementCheckRequest {
        feature,
        resources,
        attributes,
        requested: query.requested,
        current_count: query.current_count,
        explain: query.explain,
        subscription_id: query.subscription_id,
        other,
    }
}

/// Check customer entitlement
///
/// Whether the customer may consume `requested` more of a feature right now, at every level of
/// the path: the customer, and the resource named by `resources`. Each level answers from its
/// own value and usage, and all must allow. A boolean answers from its state, a value feature
/// returns its `value`, a limit counted by your app answers from its exact count, and a limit
/// counted from events from its usage. A limit that allows overage never refuses: `in_overage`
/// tells when the request goes beyond it. Usage that cannot be read refuses a limit that denies
/// with `USAGE_UNAVAILABLE` (`reason_type: unavailable`), and warns `usage_unavailable`.
/// `explain` and `current_count` take a server key.
#[utoipa::path(
    post,
    tag = "Checks",
    path = "/api/v1/customers/{id_or_alias}/entitlements/check",
    params(("id_or_alias" = String, Path, description = "Customer ID or alias")),
    request_body(content = EntitlementCheckRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Whether the customer may consume the requested units", body = EntitlementCheck),
        (status = 400, description = "Invalid request, an unknown resource key or attribute, or more than one resource", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or feature not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
        (status = 503, description = "The check could not be answered in time; retry shortly. Usage metering cannot read is answered 200 `USAGE_UNAVAILABLE`", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn check_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiPath(id_or_alias)): ApiValid<ApiPath<AliasOr<CustomerId>>>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<EntitlementCheckRequest>>,
) -> Result<Response, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;
    check_response(
        &app_state,
        authorized_state.tenant_id,
        customer.id,
        payload,
        api_key_of(&authorized_state),
    )
    .await
}

/// Check customer entitlement (GET)
///
/// The `GET` form of `POST /api/v1/customers/{id_or_alias}/entitlements/check`, with the same
/// inputs as query parameters: `resources[<key>]=<value>` and
/// `attributes[<type>.<attribute>]=<value>`.
#[utoipa::path(
    get,
    tag = "Checks",
    path = "/api/v1/customers/{id_or_alias}/entitlements/{feature_id_or_code}/check",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("feature_id_or_code" = String, Path, description = "Feature ID or code"),
        EntitlementCheckQuery,
        ("resources" = Option<std::collections::HashMap<String, String>>, Query, style = DeepObject, explode, description = "At most one resource, as `resources[<key>]=<value>`"),
        ("attributes" = Option<std::collections::HashMap<String, String>>, Query, style = DeepObject, explode, description = "Asserted attributes, as `attributes[<type>.<attribute>]=<value>`"),
        ("group" = Option<std::collections::HashMap<String, String>>, Query, style = DeepObject, explode, deprecated, description = "No longer supported: limits are not grouped, and a check sending `group[<property>]` is refused with 400"),
    ),
    responses(
        (status = 200, description = "Whether the customer may consume the requested units", body = EntitlementCheck),
        (status = 400, description = "Invalid request, an unknown resource key or attribute, more than one resource, or a `group`", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or feature not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
        (status = 503, description = "The check could not be answered in time; retry shortly. Usage metering cannot read is answered 200 `USAGE_UNAVAILABLE`", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn check_entitlement_get(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiPath(path)): ApiValid<ApiPath<CustomerFeaturePath>>,
    ApiValid(QueryParams(query)): ApiValid<QueryParams<EntitlementCheckQuery>>,
    RawQuery(raw): RawQuery,
) -> Result<Response, RestApiError> {
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(path.id_or_alias, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;
    check_response(
        &app_state,
        authorized_state.tenant_id,
        customer.id,
        check_request_of(path.feature_id_or_code, query, raw.as_deref()),
        api_key_of(&authorized_state),
    )
    .await
}

/// Report usage counted by your app
///
/// For a limit counted by your app: `set` the count, or `add` to it (negative to remove), for the
/// customer or one resource. A retried `idempotency_key` returns the stored count without applying
/// anything; a change older than the last `set` is recorded and ignored.
#[utoipa::path(
    post,
    tags = ["Customers", "Entitlements"],
    path = "/api/v1/customers/{id_or_alias}/entitlements/{feature_id_or_code}/usage",
    params(
        ("id_or_alias" = String, Path, description = "Customer ID or alias"),
        ("feature_id_or_code" = String, Path, description = "Feature ID or code"),
    ),
    request_body(content = ReportUsageRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The count after the report", body = ReportUsageResponse),
        (status = 400, description = "Not a limit counted by your app, or an invalid resource", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Customer or feature not found", body = RestErrorResponse),
        (status = 409, description = "`IDEMPOTENCY_KEY_REUSED` with another body, or `STALE_SET` older than the kept operations", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn report_usage(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiValid(ApiPath(path)): ApiValid<ApiPath<CustomerFeaturePath>>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<ReportUsageRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let tenant_id = authorized_state.tenant_id;
    let customer = app_state
        .store
        .find_customer_by_id_or_alias(path.id_or_alias, tenant_id)
        .await
        .map_err(RestApiError::from)?;
    let feature = app_state
        .store
        .get_feature_by_id_or_code(path.feature_id_or_code, tenant_id)
        .await
        .map_err(RestApiError::from)?;
    let op = match (payload.set, payload.add) {
        (Some(n), None) => AppCountOp::Set(n),
        (None, Some(n)) => AppCountOp::Add(n),
        _ => {
            return Err(RestApiError::InvalidInput(
                "send exactly one of `set` or `add`".into(),
            ));
        }
    };
    let result = app_state
        .store
        .report_app_count(
            tenant_id,
            customer.id,
            feature.id,
            AppCountWrite {
                resources: payload.resources,
                op,
                at: payload.at.map(Into::into),
                idempotency_key: payload.idempotency_key,
            },
        )
        .await
        .map_err(RestApiError::from)?;
    Ok(Json(ReportUsageResponse::from(result)))
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
pub async fn list_product_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(product_id): ApiPath<ProductId>,
) -> Result<impl IntoResponse, RestApiError> {
    let mut conn = app_state.store.get_conn().await.map_err(|e| {
        log::error!("Error getting db connection: {e}");
        RestApiError::StoreError
    })?;
    let resolved = app_state
        .store
        .resolve_entitlements_for_entity(
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

/// Create product entitlements
///
/// A product has no entitlement rows of its own: its entitlements are the feature-level
/// defaults of the features scoped to it, which is what `GET` on this path resolves. Every
/// spec must therefore target a feature belonging to `product_id`. Features that already
/// carry a default entitlement are skipped.
///
/// Specs are validated up front, but the writes are not atomic: each feature is written on
/// its own, so a failure part-way can leave earlier specs committed. Retrying is safe.
#[utoipa::path(
    post,
    tags = ["Products", "Entitlements"],
    path = "/api/v1/products/{product_id}/entitlements",
    params(("product_id" = ProductId, Path, description = "Product ID")),
    request_body(content = CreateEntitlementsRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Entitlements created", body = EntitlementListResponse),
        (status = 400, description = "Bad request, including a feature that is unknown or outside the product", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Product not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn create_product_entitlements(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(product_id): ApiPath<ProductId>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<CreateEntitlementsRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = authorized_state.tenant_id;

    app_state
        .store
        .find_product_by_id(product_id, tenant_id)
        .await
        .map_err(ApiError::from)?;

    // Every spec is checked before the first write, so a spec rejected here cannot leave the
    // specs before it committed.
    let mut validated = Vec::with_capacity(payload.entitlements.len());
    for spec in payload.entitlements {
        let feature_id = spec.feature_id;

        // Tenant-scoped read: this is the same ownership check `verify_entity_ownership`
        // performs for a Feature entity, and it also yields the product to validate against.
        let feature = app_state
            .store
            .get_feature(feature_id, tenant_id)
            .await
            .map_err(|e| body_reference_error(e, format!("feature {feature_id} not found")))?;

        if feature.product.as_ref().map(|p| p.id) != Some(product_id) {
            return Err(RestApiError::InvalidInput(format!(
                "feature {feature_id} does not belong to product {product_id}"
            ))
            .into());
        }

        let spec = spec_to_domain(spec, Tier::Default, &feature)?;
        feature
            .kind
            .check_value(&spec.value)
            .map_err(RestApiError::InvalidInput)?;

        validated.push((feature_id, spec));
    }

    // Each feature is its own entitlement entity, so these writes are still not one transaction:
    // a failure part-way leaves the earlier specs committed. Re-running skips those.
    let mut data = Vec::with_capacity(validated.len());
    for (feature_id, spec) in validated {
        let created = app_state
            .store
            .batch_create_entitlements(
                &authorized_state.as_actor(),
                tenant_id,
                EntitlementEntityId::Feature(feature_id),
                vec![spec],
            )
            .await
            .map_err(|e| {
                log::error!("Error creating product entitlements: {e}");
                ApiError::from(e)
            })?;

        data.extend(created.into_iter().map(mapping::entitlement_to_rest));
    }

    Ok((StatusCode::CREATED, Json(EntitlementListResponse { data })))
}

// ── Single entitlement ─────────────────────────────────────────

/// Get entitlement details
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/v1/entitlements/{entitlement_id}",
    params(("entitlement_id" = EntitlementId, Path, description = "Entitlement ID")),
    responses(
        (status = 200, description = "Entitlement details", body = Entitlement),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Entitlement not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn get_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(entitlement_id): ApiPath<EntitlementId>,
) -> Result<impl IntoResponse, RestApiError> {
    let entitlement = app_state
        .store
        .get_entitlement(entitlement_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error fetching entitlement: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::entitlement_to_rest(entitlement)))
}

/// Update an entitlement
///
/// Revokes the row and writes a successor under the same id, carrying the changes; the history
/// keeps both. The new value must match the feature's kind.
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
        (status = 409, description = "Edited meanwhile: read it again, or of a published plan version: write it through a draft", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn update_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(entitlement_id): ApiPath<EntitlementId>,
    ApiValid(ApiJson(payload)): ApiValid<ApiJson<UpdateEntitlementRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let tenant_id = authorized_state.tenant_id;
    let terms = payload
        .value
        .as_ref()
        .map(mapping::legacy_terms)
        .transpose()
        .map_err(RestApiError::InvalidInput)?
        .flatten();
    let mut update = mapping::entitlement_update_to_domain(payload);
    if let Some(terms) = terms {
        let row = app_state
            .store
            .get_entitlement(entitlement_id, tenant_id)
            .await
            .map_err(ApiError::from)?;
        let feature = app_state
            .store
            .get_feature(row.feature_id, tenant_id)
            .await
            .map_err(ApiError::from)?;
        let in_effect = match &update.policy_override {
            Some(Some(policy)) => policy.clone(),
            Some(None) => feature.policy_or_default(),
            None => row
                .policy_override
                .clone()
                .unwrap_or_else(|| feature.policy_or_default()),
        };
        let is_set = update.effect.unwrap_or(row.effect) == Effect::Set;
        let value = update.value.as_ref().unwrap_or(&row.value);
        if let Some(policy_override) =
            mapping::override_for_terms(&feature, &in_effect, &terms, is_set, value)
                .map_err(RestApiError::InvalidInput)?
        {
            if update.policy_override.is_some() {
                return Err(RestApiError::InvalidInput(TERMS_AND_OVERRIDE.into()).into());
            }
            update.policy_override = Some(policy_override);
        }
    }
    let entitlement = app_state
        .store
        .update_entitlement(
            &authorized_state.as_actor(),
            entitlement_id,
            tenant_id,
            update,
        )
        .await
        .map_err(|e| {
            log::error!("Error updating entitlement: {e}");
            ApiError::from(e)
        })?;

    Ok(Json(mapping::entitlement_to_rest(entitlement)))
}

/// Delete an entitlement
///
/// Revokes the row; its history stays readable.
#[utoipa::path(
    delete,
    tag = "Entitlements",
    path = "/api/v1/entitlements/{entitlement_id}",
    params(("entitlement_id" = EntitlementId, Path, description = "Entitlement ID")),
    responses(
        (status = 204, description = "Entitlement deleted"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Entitlement not found", body = RestErrorResponse),
        (status = 409, description = "Edited meanwhile: read it again, or of a published plan version: write it through a draft", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn delete_entitlement(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(entitlement_id): ApiPath<EntitlementId>,
) -> Result<impl IntoResponse, RestApiError> {
    // The store's delete drops the affected-row count, so resolve first to get the 404.
    app_state
        .store
        .get_entitlement(entitlement_id, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;

    app_state
        .store
        .delete_entitlement(
            &authorized_state.as_actor(),
            entitlement_id,
            authorized_state.tenant_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error deleting entitlement: {e}");
            meteroid_app::rest::writes::store_error(e)
        })
        .map(|_| StatusCode::NO_CONTENT)
}

/// List an entitlement's history
///
/// Every row of the entitlement, oldest first: those edits replaced, the revoked one of a deleted
/// entitlement, and the open one, with `revoked_at` and `revoked_by`.
#[utoipa::path(
    get,
    tag = "Entitlements",
    path = "/api/v1/entitlements/{entitlement_id}/history",
    params(("entitlement_id" = EntitlementId, Path, description = "Entitlement ID")),
    responses(
        (status = 200, description = "The entitlement's rows", body = EntitlementHistoryResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Entitlement not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn get_entitlement_history(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    ApiPath(entitlement_id): ApiPath<EntitlementId>,
) -> Result<impl IntoResponse, RestApiError> {
    let rows = app_state
        .store
        .get_entitlement_history(entitlement_id, authorized_state.tenant_id)
        .await
        .map_err(RestApiError::from)?;
    if rows.is_empty() {
        return Err(RestApiError::NotFound);
    }
    Ok(Json(EntitlementHistoryResponse {
        data: rows.into_iter().map(mapping::entitlement_to_rest).collect(),
    }))
}
