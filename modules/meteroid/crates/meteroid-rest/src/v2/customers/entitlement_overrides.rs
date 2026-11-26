//! `/api/v2/customers/{id_or_alias}/entitlement-overrides` (ADR-0013 §23): the rows on the
//! customer and on its stored resources, applied after every subscription's. Each is created,
//! edited (a new version under the same id, its history kept) and revoked one at a time.
use super::resources::{ResourceName, stored_resource_id};
use crate::AppState;
use crate::v2::entitlements::model::{
    Codes, Entitlement, EntitlementEffect, EntitlementInput, EntitlementValue, LevelKind, Policy,
    checked_policy, grant_value, granted_feature, spec_of,
};
use crate::v2::entitlements::rows::entitlement_id;
use crate::v2::extract::{Body, Path, Query, kept};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{
    BaseId, CustomerId, EntitlementEntityId, EntitlementId, FeatureId, TenantId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlement_fold::{Condition, Effect, Tier};
use meteroid_store::domain::entitlements::{Entitlement as DomainEntitlement, EntitlementUpdate};
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use serde::Deserialize;
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

crate::v2_list!(EntitlementOverrideList, Entitlement);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct OverrideListQuery {
    /// Only the rows of this feature (code or id).
    pub feature: Option<String>,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

/// A customer's override: a row on the customer, or with `resource` for one of its stored
/// resources only. `effect` is `raise_to` by default, so a plan granting more still wins.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateEntitlementOverride {
    /// The feature, by code or id.
    pub feature: String,
    pub value: EntitlementValue,
    /// `raise_to` (the default), `set` or `cap_at`. A text, json or unordered select value
    /// takes `set` only.
    #[serde(default)]
    pub effect: Option<EntitlementEffect>,
    /// Left out, the feature's level: per resource of its `applies_per`, else the customer.
    #[serde(default)]
    pub level: Option<LevelKind>,
    /// With `level: resource_type`: the type, by code or id.
    #[serde(default)]
    pub resource_type: Option<String>,
    /// A stored resource of the customer, `{"id": …}` or `{"type": …, "key": …}`: the override
    /// applies to it alone.
    #[serde(default)]
    pub resource: Option<ResourceName>,
    #[serde(default)]
    pub condition: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    pub valid_from: Option<ApiDateTime>,
    #[serde(default)]
    pub valid_until: Option<ApiDateTime>,
    #[serde(default)]
    pub reason: Option<String>,
    /// A limit's `set` override only: its own policy, in place of the feature's.
    #[serde(default)]
    pub policy: Option<Policy>,
}

/// A merge patch of an override: a field left out is unchanged, `null` clears it (except
/// `value` and `effect`, which cannot be cleared). The feature, the level and the resource are
/// fixed: revoke the row and create another to change them.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateEntitlementOverride {
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<EntitlementValue>)]
    pub value: Option<Option<EntitlementValue>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<EntitlementEffect>)]
    pub effect: Option<Option<EntitlementEffect>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<BTreeMap<String, Vec<String>>>)]
    pub condition: Option<Option<BTreeMap<String, Vec<String>>>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<ApiDateTime>)]
    pub valid_from: Option<Option<ApiDateTime>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<ApiDateTime>)]
    pub valid_until: Option<Option<ApiDateTime>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub reason: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<Policy>)]
    pub policy: Option<Option<Policy>>,
}

/// The open row `raw` names, refused as not found unless the customer holds it.
async fn customer_row(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    raw: &str,
) -> Result<DomainEntitlement, Problem> {
    let id = entitlement_id(raw)?;
    let row = state.store.get_entitlement(id, tenant_id).await?;
    if row.entity != EntitlementEntityId::Customer(customer_id) {
        return Err(Problem::not_found(format!("entitlement override {id}")));
    }
    Ok(row)
}

async fn render(
    state: &AppState,
    tenant_id: TenantId,
    row: DomainEntitlement,
) -> Result<Entitlement, Problem> {
    let codes = Codes::of_rows(state, tenant_id, std::slice::from_ref(&row)).await?;
    Ok(codes.entitlement(row))
}

/// List a customer's overrides
///
/// The open rows on the customer and on its stored resources, in creation order.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlement-overrides",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        OverrideListQuery,
    ),
    responses(
        (status = 200, description = "A page of the customer's overrides", body = EntitlementOverrideList),
        (status = 404, description = "The customer or the feature is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The cursor or limit is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_entitlement_overrides(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<OverrideListQuery>,
) -> Result<Json<EntitlementOverrideList>, Problem> {
    let paging = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = paging.limit()?;
    let cursor = paging.cursor()?;
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let feature: Option<FeatureId> = match query.feature.as_deref() {
        Some(raw) => Some(ids::feature(&state, auth.tenant_id, raw).await?.id),
        None => None,
    };
    let rows = state
        .store
        .list_open_entitlements_after(
            auth.tenant_id,
            EntitlementEntityId::Customer(customer.id),
            feature,
            cursor.map(|Cursor(after)| EntitlementId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |row| row.id.as_uuid());
    let codes = Codes::of_rows(&state, auth.tenant_id, &paged.data).await?;
    Ok(Json(EntitlementOverrideList {
        data: paged
            .data
            .into_iter()
            .map(|row| codes.entitlement(row))
            .collect(),
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}

/// Create a customer's override
///
/// A row on the customer, or with `resource` for one of its stored resources only. `effect` is
/// `raise_to` by default: the customer gets at least this value, and a plan granting more still
/// wins. `set` replaces what the plans give; `cap_at` limits it.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlement-overrides",
    params(("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`")),
    request_body(
        content = CreateEntitlementOverride,
        examples(
            ("customer" = (summary = "More seats until the end of the year", value = json!({
                "feature": "seats",
                "value": {"type": "limit", "limit": "25"},
                "valid_until": "2027-01-01T00:00:00Z",
                "reason": "Pilot extension"
            }))),
            ("resource" = (summary = "20 projects on one site, whatever its plan grants", value = json!({
                "feature": "projects",
                "value": {"type": "limit", "limit": "20"},
                "effect": "set",
                "resource": {"type": "site", "key": "s1"}
            }))),
        )
    ),
    responses(
        (status = 201, description = "Created", body = Entitlement, examples(
            ("customer" = (summary = "More seats until the end of the year", value = json!(examples::customer()))),
            ("resource" = (summary = "20 projects on one site", value = json!(examples::resource()))),
        )),
        (status = 404, description = "The customer is not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "`already_exists`: an open override has the same feature, level, resource and condition", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_entitlement_override(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Body(body): Body<CreateEntitlementOverride>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let feature = granted_feature(&state, tenant_id, &body.feature, "/feature").await?;
    let types = state.store.list_resource_types(tenant_id, true).await?;
    let resource_id = match &body.resource {
        Some(name) => {
            Some(stored_resource_id(&state, tenant_id, customer.id, name, "/resource").await?)
        }
        None => None,
    };
    let tier = if resource_id.is_some() {
        Tier::ResourceOverride
    } else {
        Tier::Customer
    };
    let input = EntitlementInput {
        feature: body.feature,
        value: body.value,
        effect: body.effect,
        level: body.level,
        resource_type: body.resource_type,
        condition: body.condition,
        valid_from: body.valid_from,
        valid_until: body.valid_until,
        reason: body.reason,
        policy: body.policy,
        id: None,
        owner: None,
        created_by: None,
        created_at: None,
        updated_at: None,
        revoked_at: None,
        revoked_by: None,
    };
    let spec = spec_of(&input, &feature, &types, tier, false, "")?;
    let created = state
        .store
        .create_entitlements(
            &auth.as_actor(),
            tenant_id,
            EntitlementEntityId::Customer(customer.id),
            resource_id,
            vec![spec],
        )
        .await
        .map_err(|error| {
            let mut problem = Problem::at("", error);
            if problem.code == ProblemCode::AlreadyExists {
                problem.detail = Some(
                    "An open override has the same feature, level, resource and condition: edit it"
                        .to_owned(),
                );
                problem.errors.push(FieldError {
                    path: "/feature".to_owned(),
                    code: "duplicate".to_owned(),
                    message: "An open override has the same key".to_owned(),
                });
            }
            problem
        })?;
    let row = created.into_iter().next().ok_or_else(Problem::internal)?;
    let id = row.id;
    Ok(committed_write(
        StatusCode::CREATED,
        render(&state, tenant_id, row).await,
        || serde_json::json!({ "id": id.to_string() }),
    ))
}

/// Get a customer's override
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlement-overrides/{id}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("id" = String, Path, description = "Entitlement id"),
    ),
    responses(
        (status = 200, description = "The override", body = Entitlement),
        (status = 404, description = "Not found, or revoked", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_entitlement_override(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, id)): Path<(String, String)>,
) -> Result<Json<Entitlement>, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let row = customer_row(&state, auth.tenant_id, customer.id, &id).await?;
    Ok(Json(render(&state, auth.tenant_id, row).await?))
}

/// Update a customer's override
///
/// A merge patch: the row is revoked and its successor written under the same id, its history
/// kept. A concurrent edit of the same row is `409`: read it again.
#[utoipa::path(
    patch,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlement-overrides/{id}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("id" = String, Path, description = "Entitlement id"),
    ),
    request_body(content = UpdateEntitlementOverride, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "The successor", body = Entitlement),
        (status = 404, description = "Not found, or revoked", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Another edit of the row went first", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_entitlement_override(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, id)): Path<(String, String)>,
    Body(patch): Body<UpdateEntitlementOverride>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let row = customer_row(&state, tenant_id, customer.id, &id).await?;
    let feature = state.store.get_feature(row.feature_id, tenant_id).await?;

    let value = match kept(patch.value, "/value")? {
        Some(value) => Some(
            grant_value(&value, &feature.kind)
                .map_err(|message| Problem::field("/value", "invalid", message))?,
        ),
        None => None,
    };
    let effect = kept(patch.effect, "/effect")?.map(Effect::from);
    let tier = row.tier();
    let order = feature.value_order();
    if let Some(effect) = effect
        && !tier.allowed_effects(&order).contains(&effect)
    {
        return Err(Problem::field(
            "/effect",
            "invalid",
            "An override takes raise_to, set or cap_at; an unordered value set only",
        ));
    }
    let policy = match &patch.policy {
        Some(Some(policy)) => Some(Some(checked_policy(policy, &feature.kind, "/policy")?)),
        Some(None) => Some(None),
        None => None,
    };
    let effect_after = effect.unwrap_or(row.effect);
    let policy_after = match &policy {
        Some(policy) => policy.is_some(),
        None => row.policy_override.is_some(),
    };
    if policy_after && effect_after != Effect::Set {
        return Err(Problem::field(
            "/policy",
            "invalid",
            "Only a `set` row overrides its limit's policy",
        ));
    }
    if let Some(Some(reason)) = &patch.reason {
        crate::v2::bounds::description(Some(reason), "/reason")?;
    }
    let from = match patch.valid_from {
        Some(from) => from,
        None => row.valid_from.map(Into::into),
    };
    let until = match patch.valid_until {
        Some(until) => until,
        None => row.valid_until.map(Into::into),
    };
    if let (Some(from), Some(until)) = (from, until)
        && until <= from
    {
        return Err(Problem::field(
            "/valid_until",
            "invalid",
            "valid_until is after valid_from",
        ));
    }
    let updated = state
        .store
        .update_entitlement(
            &auth.as_actor(),
            row.id,
            tenant_id,
            EntitlementUpdate {
                value,
                effect,
                condition: patch
                    .condition
                    .map(|c| c.filter(|c| !c.is_empty()).map(Condition)),
                valid_from: patch.valid_from.map(|t| t.map(Into::into)),
                valid_until: patch.valid_until.map(|t| t.map(Into::into)),
                reason: patch
                    .reason
                    .map(|r| r.map(|r| r.trim().to_owned()).filter(|r| !r.is_empty())),
                policy_override: policy,
            },
        )
        .await
        .map_err(|error| Problem::at("", error))?;
    let id: EntitlementId = updated.id;
    Ok(committed_write(
        StatusCode::OK,
        render(&state, tenant_id, updated).await,
        || serde_json::json!({ "id": id.to_string() }),
    ))
}

/// Revoke a customer's override
///
/// The row stops applying now; its history is kept (`GET /entitlements/{id}/history`).
/// Revoking it again answers `204` too.
#[utoipa::path(
    delete,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlement-overrides/{id}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("id" = String, Path, description = "Entitlement id"),
    ),
    responses(
        (status = 204, description = "Revoked"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Another edit of the row went first", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_entitlement_override(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, id)): Path<(String, String)>,
) -> Result<StatusCode, Problem> {
    let tenant_id = auth.tenant_id;
    let customer = ids::customer(&state, tenant_id, &id_or_alias).await?;
    let id = entitlement_id(&id)?;
    let versions = state
        .store
        .entitlement_history_before(tenant_id, id, None, 1)
        .await?;
    let Some(last) = versions.first() else {
        return Err(Problem::not_found(format!("entitlement override {id}")));
    };
    if last.entity != EntitlementEntityId::Customer(customer.id) {
        return Err(Problem::not_found(format!("entitlement override {id}")));
    }
    if last.revoked_at.is_none() {
        state
            .store
            .delete_entitlement(&auth.as_actor(), id, tenant_id)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The overrides the create operation's documentation shows, as they read back.
mod examples {
    use serde_json::{Value, json};

    const CUSTOMER: &str = "cus_4nM8bV2cX6zL1kJ9hG3fD5";
    const CREATED_AT: &str = "2026-11-26T10:15:00.000Z";

    pub fn customer() -> Value {
        json!({
            "id": "ent_7pQ2wE9rT4yU1iO6aS3dF8",
            "feature": "seats",
            "value": {"type": "limit", "limit": "25"},
            "effect": "raise_to",
            "level": "customer",
            "valid_until": "2027-01-01T00:00:00.000Z",
            "reason": "Pilot extension",
            "owner": {"type": "customer", "id": CUSTOMER},
            "created_by": {"type": "api_token", "id": "tkn_2aS4dF6gH8jK1lZ3xC5vB7"},
            "created_at": CREATED_AT,
            "updated_at": CREATED_AT
        })
    }

    pub fn resource() -> Value {
        json!({
            "id": "ent_9kL3mN5bV7cX2zA4sD6fG8",
            "feature": "projects",
            "value": {"type": "limit", "limit": "20"},
            "effect": "set",
            "level": "resource_type",
            "resource_type": "site",
            "resource": {"id": "res_8qW2eR4tY6uI8oP1aS3dF5", "type": "site", "key": "s1"},
            "owner": {"type": "customer", "id": CUSTOMER},
            "created_by": {"type": "api_token", "id": "tkn_2aS4dF6gH8jK1lZ3xC5vB7"},
            "created_at": CREATED_AT,
            "updated_at": CREATED_AT
        })
    }
}
