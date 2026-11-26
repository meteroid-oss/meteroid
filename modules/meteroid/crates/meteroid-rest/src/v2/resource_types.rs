//! `/api/v2/resource-types` (ADR-0013 §23, ADR-0011 §3.4): what a customer has several of (sites,
//! workspaces), one level below the customer: identifying keys, unique across the tenant's types,
//! and attributes with closed lists of values. An archived type still resolves in checks; it
//! takes no new resource nor row.
use crate::AppState;
use crate::v2::extract::{Body, Path, Query, kept};
use crate::v2::ids;
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{AliasOr, ResourceTypeId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::resources::ResourceType as DomainType;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::resources::{ResourceTypeNew, ResourceTypePatch};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ResourceType {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ResourceTypeId,
    /// Immutable: conditions name attributes as `<code>.<attribute>`, paths as `<code>=<key>`.
    pub code: String,
    pub name: String,
    pub plural_name: String,
    /// The identifying keys, the canonical one first, then its aliases.
    pub keys: Vec<String>,
    /// Each attribute with its closed list of values.
    pub attributes: BTreeMap<String, Vec<String>>,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<ApiDateTime>,
    pub created_at: ApiDateTime,
}

/// A tenant has few types: the list is not paged.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ResourceTypeList {
    pub data: Vec<ResourceType>,
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct ResourceTypeListQuery {
    /// Archived types too.
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateResourceType {
    /// Lowercase letters, digits and underscores (`site`); immutable.
    pub code: String,
    pub name: String,
    pub plural_name: String,
    /// At least one, the canonical first (`["site_id", "website_id"]`). A key another type holds
    /// is `409 key_taken`.
    pub keys: Vec<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, Vec<String>>,
}

/// A merge patch: a field left out is unchanged; the lists replace. None can be cleared.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateResourceType {
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub name: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub plural_name: Option<Option<String>>,
    /// The same clash check as on create; refused while a subscription for one of the type's
    /// resources prices a metric.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<Vec<String>>)]
    pub keys: Option<Option<Vec<String>>>,
    /// Removing a value a resource or a row's condition uses is refused, the uses listed.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<BTreeMap<String, Vec<String>>>)]
    pub attributes: Option<Option<BTreeMap<String, Vec<String>>>>,
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_resource_types, create_resource_type))
        .routes(routes!(get_resource_type, update_resource_type))
        .routes(routes!(archive_resource_type))
        .routes(routes!(unarchive_resource_type))
}

pub(crate) fn to_api(t: DomainType) -> ResourceType {
    ResourceType {
        id: t.id,
        code: t.code,
        name: t.name,
        plural_name: t.plural_name,
        keys: t.keys,
        attributes: t.attributes,
        archived: t.archived_at.is_some(),
        archived_at: t.archived_at.map(Into::into),
        created_at: t.created_at.into(),
    }
}

fn label(value: &str, path: &str) -> Result<String, Problem> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Problem::field(
            path,
            "required",
            "A resource type has a name",
        ));
    }
    crate::v2::bounds::name(value, path)?;
    Ok(value.to_owned())
}

/// A key another type holds, read at `/keys`.
fn key_taken(mut problem: Problem) -> Problem {
    if problem.code == ProblemCode::KeyTaken {
        let message = problem.detail.clone().unwrap_or_default();
        problem.errors.push(FieldError {
            path: "/keys".to_owned(),
            code: "duplicate".to_owned(),
            message,
        });
    }
    problem
}

/// List resource types
#[utoipa::path(
    get,
    tag = "Resources",
    path = "/api/v2/resource-types",
    params(ResourceTypeListQuery),
    responses(
        (status = 200, description = "The tenant's resource types, by code", body = ResourceTypeList),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_resource_types(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<ResourceTypeListQuery>,
) -> Result<Json<ResourceTypeList>, Problem> {
    let types = state
        .store
        .list_resource_types(auth.tenant_id, query.include_archived)
        .await?;
    Ok(Json(ResourceTypeList {
        data: types.into_iter().map(to_api).collect(),
    }))
}

/// Create a resource type
#[utoipa::path(
    post,
    tag = "Resources",
    path = "/api/v2/resource-types",
    request_body = CreateResourceType,
    responses(
        (status = 201, description = "Created", body = ResourceType),
        (status = 409, description = "`code_taken` at `/code`, or `key_taken` at `/keys`", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateResourceType>,
) -> Result<(StatusCode, Json<ResourceType>), Problem> {
    let code = body.code.trim().to_owned();
    let name = label(&body.name, "/name")?;
    let plural_name = label(&body.plural_name, "/plural_name")?;
    if body.keys.is_empty() {
        return Err(Problem::field(
            "/keys",
            "required",
            "A resource type has at least one key",
        ));
    }
    let created = state
        .store
        .create_resource_type(ResourceTypeNew {
            tenant_id: auth.tenant_id,
            code: code.clone(),
            name,
            plural_name,
            parent_type_id: None,
            keys: body.keys,
            attributes: body.attributes,
        })
        .await
        .map_err(|error| key_taken(Problem::at("", error).code_taken(&code)))?;
    Ok((StatusCode::CREATED, Json(to_api(created))))
}

/// Get a resource type
#[utoipa::path(
    get,
    tag = "Resources",
    path = "/api/v2/resource-types/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Resource type id or code")),
    responses(
        (status = 200, description = "The resource type", body = ResourceType),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ResourceType>, Problem> {
    let found = ids::resource_type(&state, auth.tenant_id, &id_or_code).await?;
    Ok(Json(to_api(found)))
}

/// Update a resource type
#[utoipa::path(
    patch,
    tag = "Resources",
    path = "/api/v2/resource-types/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Resource type id or code")),
    request_body(content = UpdateResourceType, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = ResourceType),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "`key_taken`, or keys or an attribute value still in use", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(patch): Body<UpdateResourceType>,
) -> Result<Json<ResourceType>, Problem> {
    let found = ids::resource_type(&state, auth.tenant_id, &id_or_code).await?;
    let name = kept(patch.name, "/name")?
        .map(|name| label(&name, "/name"))
        .transpose()?;
    let plural_name = kept(patch.plural_name, "/plural_name")?
        .map(|name| label(&name, "/plural_name"))
        .transpose()?;
    let keys = kept(patch.keys, "/keys")?;
    if keys.as_ref().is_some_and(Vec::is_empty) {
        return Err(Problem::field(
            "/keys",
            "required",
            "A resource type has at least one key",
        ));
    }
    let updated = state
        .store
        .update_resource_type(
            auth.tenant_id,
            AliasOr::Id(found.id),
            ResourceTypePatch {
                name,
                plural_name,
                parent_type_id: None,
                keys,
                attributes: kept(patch.attributes, "/attributes")?,
                archived: None,
            },
        )
        .await
        .map_err(|error| key_taken(Problem::at("", error)))?;
    Ok(Json(to_api(updated)))
}

async fn set_archived(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id_or_code: &str,
    archived: bool,
) -> Result<Json<ResourceType>, Problem> {
    let found = ids::resource_type(state, auth.tenant_id, id_or_code).await?;
    let updated = state
        .store
        .update_resource_type(
            auth.tenant_id,
            AliasOr::Id(found.id),
            ResourceTypePatch {
                archived: Some(archived),
                ..ResourceTypePatch::default()
            },
        )
        .await?;
    Ok(Json(to_api(updated)))
}

/// Archive a resource type
///
/// It still resolves in checks, its resources and rows keep working; it takes no new resource
/// nor row.
#[utoipa::path(
    post,
    tag = "Resources",
    path = "/api/v2/resource-types/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Resource type id or code")),
    responses(
        (status = 200, description = "Archived", body = ResourceType),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ResourceType>, Problem> {
    set_archived(&state, &auth, &id_or_code, true).await
}

/// Unarchive a resource type
#[utoipa::path(
    post,
    tag = "Resources",
    path = "/api/v2/resource-types/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Resource type id or code")),
    responses(
        (status = 200, description = "Unarchived", body = ResourceType),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_resource_type(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ResourceType>, Problem> {
    set_archived(&state, &auth, &id_or_code, false).await
}
