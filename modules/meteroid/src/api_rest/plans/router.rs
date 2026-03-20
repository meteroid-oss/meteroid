use super::AppState;

use crate::api_rest::QueryParams;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{PlanId, PlanVersionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::{
    FullPlanNew, OrderByRequest, PlanFilters, PlanNew, PlanPatch, PlanTrial, PlanVersionFilter,
    PlanVersionNewInternal,
};
use meteroid_store::repositories::PlansInterface;

use crate::api_rest::error::RestErrorResponse;
use crate::api_rest::model::PaginationExt;
use crate::api_rest::plans::mapping;
use crate::api_rest::plans::model::*;
use crate::errors::RestApiError;

// ── List plans ─────────────────────────────────────────────────

/// List plans
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v1/plans",
    params(PlanListRequest),
    responses(
        (status = 200, description = "List of plans", body = PlanListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_plans(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<PlanListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let filters = PlanFilters {
        search: request.search,
        filter_status: request.status.into_iter().map(Into::into).collect(),
        filter_type: request.r#type.into_iter().map(Into::into).collect(),
        filter_currency: None,
    };

    let res = app_state
        .store
        .list_full_plans(
            authorized_state.tenant_id,
            request.product_family_id,
            filters,
            request.pagination.into(),
            OrderByRequest::IdAsc,
        )
        .await
        .map_err(|e| {
            log::error!("Error handling list_plans: {e}");
            RestApiError::StoreError
        })?;

    let rest_models: Vec<Plan> = res
        .items
        .into_iter()
        .map(|fp| {
            mapping::plan_to_rest(
                fp.plan,
                fp.version,
                fp.price_components,
                fp.product_family.name,
                &fp.products,
            )
        })
        .collect();

    Ok(Json(PlanListResponse {
        data: rest_models,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

// ── Get plan ───────────────────────────────────────────────────

/// Get plan details
///
/// Retrieve a specific plan. Use `?version=draft` for the draft version,
/// `?version=2` for a specific version number, or omit for the active version.
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}",
    params(
        ("plan_id" = PlanId, Path, description = "Plan ID"),
        PlanGetQuery,
    ),
    responses(
        (status = 200, description = "Plan details", body = Plan),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn get_plan_details(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(plan_id): Path<PlanId>,
    Query(query): Query<PlanGetQuery>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let version_filter = parse_version_filter(query.version.as_deref())?;

    let fp = app_state
        .store
        .get_full_plan(plan_id, authorized_state.tenant_id, version_filter)
        .await
        .map_err(|e| {
            log::error!("Error fetching plan details: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::plan_to_rest(
        fp.plan,
        fp.version,
        fp.price_components,
        fp.product_family.name,
        &fp.products,
    )))
}

// ── Create plan ────────────────────────────────────────────────

/// Create a plan
///
/// Create a new plan with components and pricing. Set `status` to `ACTIVE` to
/// publish immediately, or `DRAFT` to stage for review.
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v1/plans",
    request_body(content = CreatePlanRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Plan created", body = Plan),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 409, description = "Conflict", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn create_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreatePlanRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let is_draft = matches!(payload.status, PlanStatusEnum::Draft);

    let components = payload
        .components
        .iter()
        .map(|c| mapping::rest_to_domain_component(c, &payload.currency))
        .collect::<Result<Vec<_>, _>>()?;

    let billing = payload.billing.unwrap_or(BillingConfig {
        net_terms: 0,
        billing_cycles: None,
        period_start_day: None,
    });

    let trial = payload.trial.map(|t| PlanTrial {
        duration_days: t.duration_days,
        trialing_plan_id: t.trialing_plan_id,
        trial_is_free: t.is_free,
    });

    let full_plan_new = FullPlanNew {
        plan: PlanNew {
            name: payload.name,
            description: payload.description,
            created_by: authorized_state.actor_id,
            tenant_id: authorized_state.tenant_id,
            product_family_id: payload.product_family_id,
            plan_type: payload.plan_type.into(),
            status: if is_draft {
                meteroid_store::domain::enums::PlanStatusEnum::Draft
            } else {
                meteroid_store::domain::enums::PlanStatusEnum::Active
            },
        },
        version: PlanVersionNewInternal {
            is_draft_version: is_draft,
            period_start_day: billing.period_start_day,
            net_terms: billing.net_terms,
            currency: Some(payload.currency),
            billing_cycles: billing.billing_cycles,
            trial,
        },
        price_components: components,
    };

    let mut full_plan = app_state
        .store
        .insert_plan(full_plan_new)
        .await
        .map_err(|e| {
            log::error!("Error creating plan: {e}");
            RestApiError::from(e)
        })?;

    // Set self_service_rank if provided (not part of PlanRowNew)
    if let Some(rank) = payload.self_service_rank {
        app_state
            .store
            .patch_published_plan(PlanPatch {
                id: full_plan.plan.id,
                tenant_id: authorized_state.tenant_id,
                name: None,
                description: None,
                active_version_id: None,
                self_service_rank: Some(Some(rank)),
            })
            .await
            .map_err(|e| {
                log::error!("Error setting self_service_rank: {e}");
                RestApiError::from(e)
            })?;
        full_plan.plan.self_service_rank = Some(rank);
    }

    // Attach add-ons
    if !payload.add_ons.is_empty() {
        attach_add_ons(
            &app_state,
            &payload.add_ons,
            full_plan.version.id,
            authorized_state.tenant_id,
        )
        .await?;
    }

    // Re-fetch to get full resolved data including products
    let fp = app_state
        .store
        .get_full_plan(
            full_plan.plan.id,
            authorized_state.tenant_id,
            if is_draft {
                PlanVersionFilter::Draft
            } else {
                PlanVersionFilter::Active
            },
        )
        .await
        .map_err(|e| {
            log::error!("Error re-fetching plan: {e}");
            RestApiError::from(e)
        })?;

    Ok((
        StatusCode::CREATED,
        Json(mapping::plan_to_rest(
            fp.plan,
            fp.version,
            fp.price_components,
            fp.product_family.name,
            &fp.products,
        )),
    ))
}

// ── Replace plan (PUT) ─────────────────────────────────────────

/// Replace a plan
///
/// Full replacement of a plan's version. On a draft plan, updates in-place.
/// On a published plan, creates a new version. Set `status` to `DRAFT` to
/// stage as a new draft without publishing.
#[utoipa::path(
    put,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    request_body(content = ReplacePlanRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Plan updated", body = Plan),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn replace_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
    Valid(Json(payload)): Valid<Json<ReplacePlanRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let publish = matches!(payload.status.as_ref(), Some(PlanStatusEnum::Active));

    let components = payload
        .components
        .iter()
        .map(|c| mapping::rest_to_domain_component(c, &payload.currency))
        .collect::<Result<Vec<_>, _>>()?;

    let billing = payload.billing.unwrap_or(BillingConfig {
        net_terms: 0,
        billing_cycles: None,
        period_start_day: None,
    });

    let trial = payload.trial.map(|t| PlanTrial {
        duration_days: t.duration_days,
        trialing_plan_id: t.trialing_plan_id,
        trial_is_free: t.is_free,
    });

    let add_ons = payload
        .add_ons
        .iter()
        .map(
            |a| meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew {
                plan_version_id: Default::default(), // will be set by store
                add_on_id: a.add_on_id,
                price_id: a.price_id,
                self_serviceable: a.self_serviceable,
                max_instances_per_subscription: a.max_instances,
                tenant_id: authorized_state.tenant_id,
            },
        )
        .collect();

    let fp = app_state
        .store
        .replace_plan_version(
            plan_id,
            authorized_state.tenant_id,
            authorized_state.actor_id,
            payload.name,
            payload.description,
            PlanVersionNewInternal {
                is_draft_version: !publish,
                period_start_day: billing.period_start_day,
                net_terms: billing.net_terms,
                currency: Some(payload.currency),
                billing_cycles: billing.billing_cycles,
                trial,
            },
            components,
            add_ons,
            publish,
        )
        .await
        .map_err(|e| {
            log::error!("Error replacing plan: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::plan_to_rest(
        fp.plan,
        fp.version,
        fp.price_components,
        fp.product_family.name,
        &fp.products,
    )))
}

// ── Patch plan ─────────────────────────────────────────────────

/// Update plan metadata
///
/// Partially update plan-level fields (name, description, self_service_rank).
/// Does not modify version-level configuration or components.
#[utoipa::path(
    patch,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    request_body(content = PatchPlanRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Plan updated", body = Plan),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn patch_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
    Valid(Json(payload)): Valid<Json<PatchPlanRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .patch_published_plan(PlanPatch {
            id: plan_id,
            tenant_id: authorized_state.tenant_id,
            name: payload.name,
            description: payload.description,
            active_version_id: None,
            self_service_rank: payload.self_service_rank,
        })
        .await
        .map_err(|e| {
            log::error!("Error patching plan: {e}");
            RestApiError::from(e)
        })?;

    // Re-fetch full plan for consistent response (try active, fall back to draft)
    let fp = match app_state
        .store
        .get_full_plan(
            plan_id,
            authorized_state.tenant_id,
            PlanVersionFilter::Active,
        )
        .await
    {
        Ok(fp) => fp,
        Err(_) => app_state
            .store
            .get_full_plan(
                plan_id,
                authorized_state.tenant_id,
                PlanVersionFilter::Draft,
            )
            .await
            .map_err(|e| {
                log::error!("Error re-fetching plan: {e}");
                RestApiError::from(e)
            })?,
    };

    Ok(Json(mapping::plan_to_rest(
        fp.plan,
        fp.version,
        fp.price_components,
        fp.product_family.name,
        &fp.products,
    )))
}

// ── Publish plan ───────────────────────────────────────────────

/// Publish a draft plan version
///
/// Publishes the current draft version, making it the active version.
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}/publish",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    responses(
        (status = 200, description = "Plan published", body = Plan),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
        (status = 409, description = "No draft version to publish", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn publish_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
) -> Result<impl IntoResponse, RestApiError> {
    let plan_with_version = app_state
        .store
        .get_plan(
            plan_id,
            authorized_state.tenant_id,
            PlanVersionFilter::Draft,
        )
        .await
        .map_err(|e| {
            log::error!("Error finding draft version: {e}");
            RestApiError::from(e)
        })?;

    let draft_version = plan_with_version.version.ok_or(RestApiError::Conflict)?;

    app_state
        .store
        .publish_plan_version(
            draft_version.id,
            authorized_state.tenant_id,
            authorized_state.actor_id,
        )
        .await
        .map_err(|e| {
            log::error!("Error publishing plan: {e}");
            RestApiError::from(e)
        })?;

    let fp = app_state
        .store
        .get_full_plan(
            plan_id,
            authorized_state.tenant_id,
            PlanVersionFilter::Active,
        )
        .await
        .map_err(|e| {
            log::error!("Error re-fetching plan: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::plan_to_rest(
        fp.plan,
        fp.version,
        fp.price_components,
        fp.product_family.name,
        &fp.products,
    )))
}

// ── Archive / Unarchive ────────────────────────────────────────

/// Archive a plan
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}/archive",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    responses(
        (status = 204, description = "Plan archived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn archive_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_plan(plan_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error archiving plan: {e}");
            RestApiError::from(e)
        })
        .map(|()| StatusCode::NO_CONTENT)
}

/// Unarchive a plan
#[utoipa::path(
    post,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}/unarchive",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    responses(
        (status = 204, description = "Plan unarchived"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn unarchive_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .unarchive_plan(plan_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error unarchiving plan: {e}");
            RestApiError::from(e)
        })
        .map(|()| StatusCode::NO_CONTENT)
}

// ── List versions ──────────────────────────────────────────────

/// List plan versions
#[utoipa::path(
    get,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}/versions",
    params(
        ("plan_id" = PlanId, Path, description = "Plan ID"),
        PlanVersionListRequest,
    ),
    responses(
        (status = 200, description = "Plan versions", body = PlanVersionListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub(crate) async fn list_plan_versions(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(plan_id): Path<PlanId>,
    Valid(Query(request)): Valid<Query<PlanVersionListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_plan_versions(
            plan_id,
            authorized_state.tenant_id,
            request.pagination.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error listing plan versions: {e}");
            RestApiError::from(e)
        })?;

    let data = res
        .items
        .iter()
        .map(mapping::plan_version_to_rest)
        .collect();

    Ok(Json(PlanVersionListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

// ── Helpers ────────────────────────────────────────────────────

fn parse_version_filter(version: Option<&str>) -> Result<PlanVersionFilter, RestApiError> {
    match version {
        None => Ok(PlanVersionFilter::Active),
        Some("draft") => Ok(PlanVersionFilter::Draft),
        Some(s) => s
            .parse::<i32>()
            .map(PlanVersionFilter::Version)
            .map_err(|_| {
                RestApiError::InvalidInput(format!(
                    "Invalid version filter '{}': use 'draft' or a version number",
                    s
                ))
            }),
    }
}

async fn attach_add_ons(
    app_state: &AppState,
    add_ons: &[PlanAddOnInput],
    plan_version_id: PlanVersionId,
    tenant_id: TenantId,
) -> Result<(), RestApiError> {
    use meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew;
    use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;

    for addon in add_ons {
        app_state
            .store
            .attach_add_on_to_plan_version(PlanVersionAddOnNew {
                plan_version_id,
                add_on_id: addon.add_on_id,
                price_id: addon.price_id,
                self_serviceable: addon.self_serviceable,
                max_instances_per_subscription: addon.max_instances,
                tenant_id,
            })
            .await
            .map_err(|e| {
                log::error!("Error attaching add-on: {e}");
                RestApiError::from(e)
            })?;
    }
    Ok(())
}
