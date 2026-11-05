use super::AppState;

use meteroid_rest_common::QueryParams;

use crate::plans::mapping;
use crate::plans::model::*;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::{CreditUnitId, PlanId, PlanVersionId, ProductId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_app::features::minimum_commitments_open;
use meteroid_rest_common::error::RestErrorResponse;
use meteroid_rest_common::model::{PaginationExt, validate_order_by};
use meteroid_store::domain::{
    FullPlanNew, PlanFilters, PlanNew, PlanPatch, PlanTrial, PlanVersionFilter,
    PlanVersionNewInternal,
};
use meteroid_store::repositories::PlansInterface;
use meteroid_store::repositories::credit_units::CreditUnitsInterface;
use meteroid_store::repositories::plans::PlansInterfaceAuto;

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
pub async fn list_plans(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<PlanListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let filters = PlanFilters {
        search: request.search,
        filter_status: request.status.into_iter().map(Into::into).collect(),
        filter_type: request.plan_type.into_iter().map(Into::into).collect(),
        filter_currency: None,
    };

    let order_by = validate_order_by(
        &request.order_by,
        &["name", "status", "plan_type", "created_at"],
        "created_at.desc",
    )
    .map_err(RestApiError::InvalidInput)?;

    let res = app_state
        .store
        .list_full_plans(
            authorized_state.tenant_id,
            request.product_family_id,
            filters,
            request.pagination.into(),
            Some(order_by),
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
        .collect::<Result<_, _>>()?;

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
pub async fn get_plan_details(
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
    )?))
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
pub async fn create_plan(
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
    ensure_credit_pricing_allowed(
        app_state.feature_resolver.as_ref(),
        authorized_state.organization_id,
        components
            .iter()
            .any(|c| c.product_ref.creates_credit_product()),
    )
    .await?;

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
            entitlements: entitlement_specs(payload.entitlements)?,
            tax_inclusive: payload.tax_inclusive,
            credit_exhausted_rule: payload
                .credit_exhausted_rule
                .map(mapping::credit_exhausted_rule_from_rest)
                .transpose()?,
        },
        price_components: components,
    };

    app_state
        .store
        .ensure_credit_rule_enforceable(full_plan_new.version.credit_exhausted_rule.as_ref())
        .map_err(RestApiError::from)?;
    let mut full_plan = app_state
        .store
        .insert_plan(authorized_state.as_actor(), full_plan_new)
        .await
        .map_err(|e| {
            log::error!("Error creating plan: {e}");
            RestApiError::from(e)
        })?;

    // Set self_service_rank if provided (not part of PlanRowNew)
    if let Some(rank) = payload.self_service_rank {
        app_state
            .store
            .patch_published_plan(
                authorized_state.as_actor(),
                PlanPatch {
                    id: full_plan.plan.id,
                    tenant_id: authorized_state.tenant_id,
                    name: None,
                    description: None,
                    active_version_id: None,
                    self_service_rank: Some(Some(rank)),
                    issues_license: None,
                },
            )
            .await
            .map_err(|e| {
                log::error!("Error setting self_service_rank: {e}");
                RestApiError::from(e)
            })?;
        full_plan.plan.self_service_rank = Some(rank);
    }

    let created_entitlements = std::mem::take(&mut full_plan.version.entitlements);

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
    let mut fp = app_state
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

    fp.version.entitlements = created_entitlements;

    Ok((
        StatusCode::CREATED,
        Json(mapping::plan_to_rest(
            fp.plan,
            fp.version,
            fp.price_components,
            fp.product_family.name,
            &fp.products,
        )?),
    ))
}

// ── Replace plan (PUT) ─────────────────────────────────────────

/// Replace a plan
///
/// Full replacement of a plan's version. On a draft plan, updates in-place.
/// On a published plan, creates a new version. Set `status` to `DRAFT` to
/// stage as a new draft without publishing. A usage or capacity component priced in a credit
/// unit must be sent with its `credit_unit`, or the request is refused: omitting it would bill
/// its credits as money.
#[utoipa::path(
    put,
    tag = "Plans",
    path = "/api/v1/plans/{plan_id}",
    params(("plan_id" = PlanId, Path, description = "Plan ID")),
    request_body(content = ReplacePlanRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Plan updated", body = Plan),
        (status = 400, description = "Bad request, or a credit-unit component sent without its `credit_unit`", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Minimum commitments are not included in the plan: introducing or changing one is refused", body = RestErrorResponse),
        (status = 404, description = "Plan not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn replace_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
    Valid(Json(payload)): Valid<Json<ReplacePlanRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let publish = matches!(payload.status.as_ref(), Some(PlanStatusEnum::Active));

    let credit_exhausted_rule = payload
        .credit_exhausted_rule
        .clone()
        .map(mapping::credit_exhausted_rule_from_rest)
        .transpose()?;
    app_state
        .store
        .ensure_credit_rule_enforceable(credit_exhausted_rule.as_ref())
        .map_err(RestApiError::from)?;

    ensure_credit_units_kept(
        &app_state,
        authorized_state.tenant_id,
        plan_id,
        &payload.components,
    )
    .await?;

    let components = payload
        .components
        .iter()
        .map(|c| mapping::rest_to_domain_component(c, &payload.currency))
        .collect::<Result<Vec<_>, _>>()?;
    ensure_credit_pricing_allowed(
        app_state.feature_resolver.as_ref(),
        authorized_state.organization_id,
        components
            .iter()
            .any(|c| c.product_ref.creates_credit_product()),
    )
    .await?;

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

    let minimum = match payload.minimum_commitment {
        None => None,
        Some(mc) => {
            use meteroid_store::domain::plan_minimums::{
                ReplacePlanMinimumInput, ReplacePlanMinimumScope,
            };
            let amount_subunit = parse_amount_to_subunit(&mc.amount, &payload.currency)?;
            let scope = match mc.scope {
                MinimumCommitmentInputScope::AllComponents(_) => {
                    ReplacePlanMinimumScope::AllComponents
                }
                MinimumCommitmentInputScope::Components(c) => {
                    ReplacePlanMinimumScope::Products(c.component_names)
                }
            };
            Some(ReplacePlanMinimumInput {
                amount_subunit,
                scope,
            })
        }
    };
    if let Some(minimum) = minimum.as_ref()
        && !minimum_commitments_open(
            app_state.feature_resolver.as_ref(),
            authorized_state.organization_id,
        )
        .await
    {
        ensure_minimum_kept(
            &app_state,
            authorized_state.tenant_id,
            plan_id,
            minimum,
            &components,
        )
        .await?;
    }

    let fp = app_state
        .store
        .replace_plan_version(
            authorized_state.as_actor(),
            plan_id,
            authorized_state.tenant_id,
            payload.name,
            payload.description,
            PlanVersionNewInternal {
                is_draft_version: !publish,
                period_start_day: billing.period_start_day,
                net_terms: billing.net_terms,
                currency: Some(payload.currency),
                billing_cycles: billing.billing_cycles,
                trial,
                entitlements: entitlement_specs(payload.entitlements)?,
                tax_inclusive: payload.tax_inclusive,
                credit_exhausted_rule,
            },
            components,
            add_ons,
            publish,
            minimum,
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
    )?))
}

/// Refuses a replacement that would reprice a credit-unit component in money: generated SDKs
/// drop the fields they don't know, so a GET → PUT round trip may silently lose `credit_unit`.
async fn ensure_credit_units_kept(
    app_state: &AppState,
    tenant_id: TenantId,
    plan_id: PlanId,
    components: &[PriceComponentInput],
) -> Result<(), RestApiError> {
    let overview = app_state
        .store
        .get_plan_overview(plan_id, tenant_id)
        .await
        .map_err(RestApiError::from)?;
    let mut filters = Vec::new();
    if overview.active_version.is_some() {
        filters.push(PlanVersionFilter::Active);
    }
    if overview.has_draft_version {
        filters.push(PlanVersionFilter::Draft);
    }
    let mut priced: Vec<(ProductId, String, CreditUnitId)> = Vec::new();
    for filter in filters {
        let current = app_state
            .store
            .get_full_plan(plan_id, tenant_id, filter)
            .await
            .map_err(RestApiError::from)?;
        for component in &current.price_components {
            let unit = component
                .prices
                .iter()
                .find_map(|price| price.credit_unit.as_ref());
            if let (Some(product_id), Some(unit)) = (component.product_id, unit) {
                priced.push((product_id, component.name.clone(), unit.unit_id));
            }
        }
    }
    for (product_id, name, unit_id) in priced {
        let dropped = components.iter().any(|input| {
            input.product_id == Some(product_id)
                && !matches!(
                    &input.fee,
                    Fee::Usage(UsagePlanFee {
                        credit_unit: Some(_),
                        ..
                    }) | Fee::Capacity(CapacityPlanFee {
                        credit_unit: Some(_),
                        ..
                    })
                )
        });
        if dropped {
            let unit = app_state
                .store
                .get_credit_unit(tenant_id, unit_id)
                .await
                .map_err(RestApiError::from)?;
            return Err(RestApiError::InvalidInput(format!(
                "Component '{name}' is priced in {}; send credit_unit to keep it, or remove the component",
                unit.plural_name
            )));
        }
    }
    Ok(())
}

/// Without the `minimum_commitments` gate, a replacement may only resend the minimum of the
/// plan's draft or active version. A component on a new product is never in a stored scope.
async fn ensure_minimum_kept(
    app_state: &AppState,
    tenant_id: TenantId,
    plan_id: PlanId,
    minimum: &meteroid_store::domain::plan_minimums::ReplacePlanMinimumInput,
    components: &[meteroid_store::domain::price_components::PriceComponentNewInternal],
) -> Result<(), RestApiError> {
    use meteroid_store::domain::plan_minimums::{MinimumScope, ReplacePlanMinimumScope};
    use meteroid_store::domain::price_components::ProductRef;

    let scope = match &minimum.scope {
        ReplacePlanMinimumScope::AllComponents => Some(MinimumScope::AllComponents),
        ReplacePlanMinimumScope::Products(names) => names
            .iter()
            .map(|name| {
                components
                    .iter()
                    .find(|c| &c.name == name)
                    .and_then(|c| match c.product_ref {
                        ProductRef::Existing(product_id) => Some(product_id),
                        ProductRef::New { .. } => None,
                    })
            })
            .collect::<Option<Vec<_>>>()
            .map(MinimumScope::Products),
    };
    if let Some(scope) = scope {
        let overview = app_state
            .store
            .get_plan_overview(plan_id, tenant_id)
            .await
            .map_err(RestApiError::from)?;
        let mut filters = Vec::new();
        if overview.active_version.is_some() {
            filters.push(PlanVersionFilter::Active);
        }
        if overview.has_draft_version {
            filters.push(PlanVersionFilter::Draft);
        }
        for filter in filters {
            let stored = app_state
                .store
                .get_plan(plan_id, tenant_id, filter)
                .await
                .map_err(RestApiError::from)?
                .version
                .and_then(|v| v.minimum);
            if stored.is_some_and(|stored| {
                stored.amount_subunit == minimum.amount_subunit && stored.scope.same_as(&scope)
            }) {
                return Ok(());
            }
        }
    }
    Err(RestApiError::Forbidden)
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
pub async fn patch_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
    Valid(Json(payload)): Valid<Json<PatchPlanRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .patch_published_plan(
            authorized_state.as_actor(),
            PlanPatch {
                id: plan_id,
                tenant_id: authorized_state.tenant_id,
                name: payload.name,
                description: payload.description,
                active_version_id: None,
                self_service_rank: payload.self_service_rank,
                issues_license: None,
            },
        )
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
        Err(err) => {
            if matches!(
                err.current_context(),
                meteroid_store::errors::StoreError::ValueNotFound(_)
            ) {
                app_state
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
                    })?
            } else {
                log::error!("Error fetching active plan version: {err}");
                return Err(RestApiError::from(err));
            }
        }
    };

    Ok(Json(mapping::plan_to_rest(
        fp.plan,
        fp.version,
        fp.price_components,
        fp.product_family.name,
        &fp.products,
    )?))
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
pub async fn publish_plan(
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
            authorized_state.as_actor(),
            draft_version.id,
            authorized_state.tenant_id,
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
    )?))
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
pub async fn archive_plan(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_id): Path<PlanId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .archive_plan(
            authorized_state.as_actor(),
            plan_id,
            authorized_state.tenant_id,
        )
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
pub async fn unarchive_plan(
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
pub async fn list_plan_versions(
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
        .collect::<Result<_, _>>()?;

    Ok(Json(PlanVersionListResponse {
        data,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

// ── Plan minimum commitment ────────────────────────────────────

/// Set or replace the plan-level minimum commitment for a draft plan version.
#[utoipa::path(
    put,
    tag = "Plans",
    path = "/api/v1/plans/versions/{plan_version_id}/minimum",
    params(("plan_version_id" = PlanVersionId, Path, description = "Plan version ID")),
    request_body = MinimumCommitment,
    responses(
        (status = 200, description = "Minimum commitment saved", body = MinimumCommitment),
        (status = 400, description = "Invalid input", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 403, description = "Minimum commitments are not included in the plan: introducing or changing one is refused", body = RestErrorResponse),
        (status = 404, description = "Plan version not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn set_plan_minimum(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_version_id): Path<PlanVersionId>,
    Json(body): Json<MinimumCommitment>,
) -> Result<impl IntoResponse, RestApiError> {
    use meteroid_store::domain::plan_minimums::MinimumScope;
    use std::str::FromStr;

    // Resolve the plan version's currency so we can convert the major-unit amount string
    // into a subunit i64. Loading PlanWithVersion validates tenant + draft-only is then
    // enforced by the store itself under a row lock.
    let plan = app_state
        .store
        .get_plan_by_version_id(plan_version_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error loading plan version: {e}");
            RestApiError::from(e)
        })?;
    let currency = plan
        .version
        .as_ref()
        .map(|v| v.currency.as_str())
        .ok_or_else(|| RestApiError::InvalidInput("plan version not found".to_string()))?;
    let amount_subunit = parse_amount_to_subunit(&body.amount, currency)?;

    let scope = match body.scope {
        MinimumCommitmentScope::AllComponents(_) => MinimumScope::AllComponents,
        MinimumCommitmentScope::Products(p) => {
            let ids = p
                .product_ids
                .iter()
                .map(|s| {
                    common_domain::ids::ProductId::from_str(s)
                        .map_err(|e| RestApiError::InvalidInput(format!("Invalid id: {}", e)))
                })
                .collect::<Result<Vec<_>, _>>()?;
            MinimumScope::Products(ids)
        }
    };

    if !minimum_commitments_open(
        app_state.feature_resolver.as_ref(),
        authorized_state.organization_id,
    )
    .await
    {
        let kept = plan
            .version
            .as_ref()
            .and_then(|v| v.minimum.as_ref())
            .is_some_and(|stored| {
                stored.amount_subunit == amount_subunit && stored.scope.same_as(&scope)
            });
        if !kept {
            return Err(RestApiError::Forbidden);
        }
    }

    let saved = app_state
        .store
        .set_plan_minimum(
            plan_version_id,
            authorized_state.tenant_id,
            amount_subunit,
            scope,
        )
        .await
        .map_err(|e| {
            log::error!("Error setting plan minimum: {e}");
            RestApiError::from(e)
        })?;

    Ok(Json(mapping::plan_minimum_to_rest(&saved, currency)))
}

/// Remove the plan-level minimum commitment for a draft plan version.
#[utoipa::path(
    delete,
    tag = "Plans",
    path = "/api/v1/plans/versions/{plan_version_id}/minimum",
    params(("plan_version_id" = PlanVersionId, Path, description = "Plan version ID")),
    responses(
        (status = 204, description = "Minimum commitment removed"),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Plan version not found", body = RestErrorResponse),
    ),
    security(("bearer_auth" = []))
)]
#[axum::debug_handler]
pub async fn delete_plan_minimum(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    State(app_state): State<AppState>,
    Path(plan_version_id): Path<PlanVersionId>,
) -> Result<impl IntoResponse, RestApiError> {
    app_state
        .store
        .clear_plan_minimum(plan_version_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error clearing plan minimum: {e}");
            RestApiError::from(e)
        })
        .map(|()| StatusCode::NO_CONTENT)
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

/// Parse a major-unit amount string (e.g. "100.00") into a subunit i64 using the currency's
/// precision. Rejects unknown currencies and amounts with more decimal places than the
/// currency supports (defense in depth — the frontend also validates).
pub fn parse_amount_to_subunit(amount: &str, currency: &str) -> Result<i64, RestApiError> {
    use common_utils::decimals::ToSubunit;
    use meteroid_store::constants::Currencies;
    use std::str::FromStr;

    let precision = Currencies::resolve_currency_precision(currency)
        .ok_or_else(|| RestApiError::InvalidInput(format!("Unknown currency '{}'", currency)))?;

    let decimal = rust_decimal::Decimal::from_str(amount)
        .map_err(|e| RestApiError::InvalidInput(format!("Invalid amount '{}': {}", amount, e)))?;

    if decimal.scale() > u32::from(precision) {
        return Err(RestApiError::InvalidInput(format!(
            "Amount '{}' has more decimal places than {} supports ({})",
            amount, currency, precision
        )));
    }

    let subunit = decimal.to_subunit_opt(precision).ok_or_else(|| {
        RestApiError::InvalidInput(format!(
            "Amount '{}' overflows i64 subunit range for currency {}",
            amount, currency
        ))
    })?;

    // Every caller parses a minimum-commitment amount, which must be strictly positive
    // (matches the gRPC mapper and the `CHECK (amount_subunit > 0)` DB constraint).
    if subunit <= 0 {
        return Err(RestApiError::InvalidInput(format!(
            "Amount '{}' must be greater than zero",
            amount
        )));
    }

    Ok(subunit)
}

fn entitlement_specs(
    specs: Vec<crate::entitlements::model::EntitlementSpecRequest>,
) -> Result<Vec<meteroid_store::domain::entitlements::EntitlementSpec>, RestApiError> {
    specs
        .into_iter()
        .map(crate::entitlements::mapping::entitlement_spec_to_domain)
        .collect::<Result<_, _>>()
        .map_err(RestApiError::InvalidInput)
}
