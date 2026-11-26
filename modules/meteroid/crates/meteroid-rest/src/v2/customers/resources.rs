//! `/api/v2/customers/{id_or_alias}/resources/{type}/{key}` (ADR-0013 §25, ADR-0011 §3.4): a
//! customer's stored resources, by their type and key value, listed by keyset in SQL. A resource
//! is stored for an override of it alone, a subscription for it, attributes the caller must not
//! assert, and listing; one named only in a check needs none. It stays with its customer.
//!
//! A request names a resource one way everywhere in v2 ([`ResourceName`]): `{id}`, or its type's
//! code and key value `{type, key}`; a response reads it as [`ResourceRef`].
use crate::AppState;
use crate::v2::Reader;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::{Problem, ProblemCode};
pub use crate::v2::refs::ResourceRef;
use crate::v2::refs::ResourceTypeRef;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{AliasOr, BaseId, CustomerId, ResourceId, ResourceTypeId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_store::domain::Customer;
use meteroid_store::domain::resources::{Resource as DomainResource, ResourceInput, ResourceType};
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::resources::ResourceUpsert;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

// ── Naming a resource ───────────────────────────────────────────────

/// A customer's resource: `{"id": "res_…"}` for a stored one, or its type and key value,
/// `{"type": "site", "key": "s1"}`, which a check, a snapshot or a usage report may name without
/// the resource being stored. Send `id`, or `type` and `key`.
#[derive(Deserialize, ToSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ResourceName {
    /// A stored resource's id.
    #[serde(default)]
    pub id: Option<String>,
    /// With `key`: the resource type's code, or its id.
    #[serde(default, rename = "type")]
    pub resource_type: Option<String>,
    /// With `type`: the resource's key value (`s1`).
    #[serde(default)]
    pub key: Option<String>,
}

/// A resource as a request names it, once checked to be one of the two forms.
enum Named<'a> {
    Id(&'a str),
    Key {
        resource_type: &'a str,
        key: &'a str,
    },
}

impl ResourceName {
    pub fn id(id: impl Into<String>) -> Self {
        ResourceName {
            id: Some(id.into()),
            ..ResourceName::default()
        }
    }

    pub fn key(resource_type: impl Into<String>, key: impl Into<String>) -> Self {
        ResourceName {
            id: None,
            resource_type: Some(resource_type.into()),
            key: Some(key.into()),
        }
    }

    /// The form named, refused at `pointer` unless it is `id` alone or `type` with `key`.
    fn named(&self, pointer: &str) -> Result<Named<'_>, Problem> {
        match (
            self.id.as_deref(),
            self.resource_type.as_deref(),
            self.key.as_deref(),
        ) {
            (Some(id), None, None) => Ok(Named::Id(id)),
            (None, Some(resource_type), Some(key)) => Ok(Named::Key { resource_type, key }),
            (Some(_), _, _) => Err(Problem::field(
                pointer,
                "invalid",
                "Name the resource by its `id`, or by its `type` and `key`, not both",
            )),
            (None, None, None) => Err(Problem::field(
                pointer,
                "required",
                "Name the resource by its `id`, or by its `type` and `key`",
            )),
            (None, None, Some(_)) => Err(Problem::field(
                format!("{pointer}/type"),
                "required",
                "A resource named by its `key` names its `type`",
            )),
            (None, Some(_), None) => Err(Problem::field(
                format!("{pointer}/key"),
                "required",
                "A resource named by its `type` names its `key`",
            )),
        }
    }
}

pub(crate) fn resource_ref(resource: &DomainResource, types: &[ResourceType]) -> ResourceRef {
    ResourceRef {
        id: resource.id,
        resource_type: types
            .iter()
            .find(|t| t.id == resource.resource_type_id)
            .map_or_else(|| resource.resource_type_id.to_string(), |t| t.code.clone()),
        key: resource.key_value.clone(),
        name: resource.name.clone(),
    }
}

/// The resources of `ids` as responses read them; one deleted since is left out.
pub(crate) async fn resource_refs(
    state: &impl Reader,
    tenant_id: TenantId,
    ids: impl IntoIterator<Item = ResourceId>,
) -> Result<HashMap<ResourceId, ResourceRef>, Problem> {
    let ids: Vec<ResourceId> = ids
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let resources = state.store().get_resources_by_ids(tenant_id, &ids).await?;
    let types = state.store().list_resource_types(tenant_id, true).await?;
    Ok(resources
        .iter()
        .map(|resource| (resource.id, resource_ref(resource, &types)))
        .collect())
}

fn type_named<'a>(
    types: &'a [ResourceType],
    raw: &str,
    pointer: &str,
) -> Result<&'a ResourceType, Problem> {
    let raw = raw.trim();
    let id = ids::parse_id::<ResourceTypeId>(raw);
    types
        .iter()
        .find(|t| Some(t.id) == id || t.code == raw)
        .ok_or_else(|| {
            Problem::field(
                format!("{pointer}/type"),
                "not_found",
                format!("No resource type '{raw}'"),
            )
        })
}

fn no_such_resource(pointer: &str, what: impl std::fmt::Display) -> Problem {
    Problem::field(
        pointer,
        "not_found",
        format!("The customer has no resource {what}"),
    )
}

/// The customer's stored resource `name` names, refused at `pointer`.
pub(crate) async fn stored_resource(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    name: &ResourceName,
    pointer: &str,
) -> Result<DomainResource, Problem> {
    match name.named(pointer)? {
        Named::Id(raw) => {
            let raw = raw.trim();
            let id = ids::parse_id::<ResourceId>(raw).ok_or_else(|| {
                Problem::field(pointer, "invalid", format!("'{raw}' is not a resource id"))
            })?;
            let resource = state
                .store
                .get_resources_by_ids(tenant_id, &[id])
                .await?
                .into_iter()
                .find(|r| r.customer_id == customer_id)
                .ok_or_else(|| no_such_resource(pointer, id))?;
            Ok(resource)
        }
        Named::Key { resource_type, key } => {
            let types = state.store.list_resource_types(tenant_id, true).await?;
            let resource_type = type_named(&types, resource_type, pointer)?;
            state
                .store
                .find_resource(tenant_id, customer_id, AliasOr::Id(resource_type.id), key)
                .await
                .map_err(|error| {
                    let problem = Problem::from(error);
                    if problem.code == ProblemCode::NotFound {
                        no_such_resource(pointer, format!("{} '{key}'", resource_type.code))
                    } else {
                        problem
                    }
                })
        }
    }
}

/// The id of the customer's stored resource `name` names, refused at `pointer`.
pub(crate) async fn stored_resource_id(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    name: &ResourceName,
    pointer: &str,
) -> Result<ResourceId, Problem> {
    Ok(
        stored_resource(state, tenant_id, customer_id, name, pointer)
            .await?
            .id,
    )
}

/// What the store reads a check's, a snapshot's or a usage report's resource as: its type's
/// canonical key and the key value, stored or not, with the asserted `attributes`.
pub(crate) async fn resource_input(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    name: Option<&ResourceName>,
    attributes: BTreeMap<String, String>,
    pointer: &str,
) -> Result<ResourceInput, Problem> {
    let Some(name) = name else {
        return Ok(ResourceInput {
            resources: BTreeMap::new(),
            attributes,
        });
    };
    let types = state.store.list_resource_types(tenant_id, true).await?;
    let (resource_type, key_value) = match name.named(pointer)? {
        Named::Id(_) => {
            let stored = stored_resource(state, tenant_id, customer_id, name, pointer).await?;
            let resource_type = types
                .iter()
                .find(|t| t.id == stored.resource_type_id)
                .ok_or_else(Problem::internal)?;
            (resource_type, stored.key_value)
        }
        Named::Key { resource_type, key } => {
            let key = key.trim();
            if key.is_empty() {
                return Err(Problem::field(
                    format!("{pointer}/key"),
                    "invalid",
                    "A key value is not empty",
                ));
            }
            (type_named(&types, resource_type, pointer)?, key.to_owned())
        }
    };
    let canonical = resource_type
        .keys
        .first()
        .ok_or_else(Problem::internal)?
        .clone();
    Ok(ResourceInput {
        resources: BTreeMap::from([(canonical, key_value)]),
        attributes,
    })
}

/// The resource a query names, as `resource=res_…` or `resource[type]=<code>&resource[key]=<value>`.
/// v1's `resources[<key>]=<value>` is refused rather than read as the customer.
pub(crate) fn query_resource(raw: Option<&str>) -> Result<Option<ResourceName>, Problem> {
    let mut id: Option<String> = None;
    let mut resource_type: Option<String> = None;
    let mut key: Option<String> = None;
    for (name, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            "resource" => id = Some(value.into_owned()),
            "resource[type]" => resource_type = Some(value.into_owned()),
            "resource[key]" => key = Some(value.into_owned()),
            other if other.starts_with("resource[") => {
                return Err(Problem::field(
                    other,
                    "invalid",
                    "A resource is `resource[type]` and `resource[key]`, or `resource` (its id)",
                ));
            }
            other if other == "resources" || other.starts_with("resources[") => {
                return Err(Problem::field(
                    "resources",
                    "invalid",
                    "Name the resource as `resource=res_…`, or `resource[type]=<code>&resource[key]=<value>`",
                ));
            }
            _ => {}
        }
    }
    match (id, resource_type, key) {
        (None, None, None) => Ok(None),
        (Some(id), None, None) => Ok(Some(ResourceName::id(id))),
        (Some(_), _, _) => Err(Problem::field(
            "resource",
            "invalid",
            "Name the resource by its id or by its type and key, not both",
        )),
        (None, Some(resource_type), Some(key)) => Ok(Some(ResourceName::key(resource_type, key))),
        (None, None, Some(_)) => Err(Problem::field(
            "resource[type]",
            "required",
            "A resource named by its key names its type",
        )),
        (None, Some(_), None) => Err(Problem::field(
            "resource[key]",
            "required",
            "A resource named by its type names its key",
        )),
    }
}

// ── Stored resources ────────────────────────────────────────────────

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Resource {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ResourceId,
    #[serde(rename = "type")]
    pub resource_type: ResourceTypeRef,
    /// The value of the type's keys (`s1`).
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `{"<attribute>": "<value>"}`, without the type's code; they win over asserted ones.
    pub attributes: BTreeMap<String, String>,
    /// An archived resource still resolves in checks.
    pub archived: bool,
}

/// The resource's name and attributes, replaced whole; its archive is unchanged.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct PutResource {
    #[serde(default)]
    pub name: Option<String>,
    /// From the type's closed lists, without the type's code.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

fn to_api(resource: DomainResource, resource_type: &ResourceType) -> Resource {
    Resource {
        id: resource.id,
        resource_type: ResourceTypeRef {
            id: resource_type.id,
            code: resource_type.code.clone(),
        },
        key: resource.key_value,
        name: resource.name,
        attributes: resource.attributes,
        archived: resource.archived_at.is_some(),
    }
}

/// The customer, its resource's type and the stored resource, when there is one.
async fn located(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id_or_alias: &str,
    resource_type: &str,
    key: &str,
) -> Result<(Customer, ResourceType, Option<DomainResource>), Problem> {
    let customer = ids::customer(state, auth.tenant_id, id_or_alias).await?;
    let resource_type = ids::resource_type(state, auth.tenant_id, resource_type).await?;
    let found = state
        .store
        .find_resource(
            auth.tenant_id,
            customer.id,
            AliasOr::Id(resource_type.id),
            key,
        )
        .await;
    let found = match found {
        Ok(resource) => Some(resource),
        Err(error) => {
            let problem = Problem::from(error);
            if problem.code != ProblemCode::NotFound {
                return Err(problem);
            }
            None
        }
    };
    Ok((customer, resource_type, found))
}

fn not_found(resource_type: &ResourceType, key: &str) -> Problem {
    Problem::not_found(format!(
        "The customer has no {} '{key}'",
        resource_type.code
    ))
}

/// Get a customer's resource
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/resources/{type}/{key}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("type" = String, Path, description = "Resource type code or id"),
        ("key" = String, Path, description = "The resource's key value"),
    ),
    responses(
        (status = 200, description = "The resource", body = Resource),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, resource_type, key)): Path<(String, String, String)>,
) -> Result<Json<Resource>, Problem> {
    let (_, resource_type, found) =
        located(&state, &auth, &id_or_alias, &resource_type, &key).await?;
    let resource = found.ok_or_else(|| not_found(&resource_type, &key))?;
    Ok(Json(to_api(resource, &resource_type)))
}

/// Store a customer's resource
///
/// Creates the resource (`201`) or replaces its name and attributes (`200`); its archive is
/// unchanged. Up to 10,000 per customer and type.
#[utoipa::path(
    put,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/resources/{type}/{key}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("type" = String, Path, description = "Resource type code or id"),
        ("key" = String, Path, description = "The resource's key value"),
    ),
    request_body = PutResource,
    responses(
        (status = 200, description = "Replaced", body = Resource),
        (status = 201, description = "Created", body = Resource),
        (status = 404, description = "The customer or the resource type is not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "`resource_limit_reached`: the customer holds the most resources of the type allowed", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "An undeclared attribute or value, an invalid key value, or an archived type (`archived`)", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn put_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, resource_type, key)): Path<(String, String, String)>,
    Body(body): Body<PutResource>,
) -> Result<(StatusCode, Json<Resource>), Problem> {
    let (customer, resource_type, found) =
        located(&state, &auth, &id_or_alias, &resource_type, &key).await?;
    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| crate::v2::bounds::name(name, "/name").map(|()| name.to_owned()))
        .transpose()?;
    let stored = state
        .store
        .upsert_resource(ResourceUpsert {
            tenant_id: auth.tenant_id,
            customer_id: customer.id,
            resource_type: AliasOr::Id(resource_type.id),
            key_value: key,
            name,
            attributes: body.attributes,
            archived: found.as_ref().is_some_and(|r| r.archived_at.is_some()),
        })
        .await
        .map_err(|error| Problem::at("/attributes", error))?;
    let status = if found.is_some() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(to_api(stored, &resource_type))))
}

async fn set_archived(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    (id_or_alias, resource_type, key): (String, String, String),
    archived: bool,
) -> Result<Json<Resource>, Problem> {
    let (_, resource_type, found) =
        located(state, auth, &id_or_alias, &resource_type, &key).await?;
    let resource = found.ok_or_else(|| not_found(&resource_type, &key))?;
    let updated = state
        .store
        .set_resource_archived(auth.tenant_id, resource.id, archived)
        .await?;
    Ok(Json(to_api(updated, &resource_type)))
}

/// Archive a customer's resource
///
/// It still resolves in checks, and what names it keeps working.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/resources/{type}/{key}/archive",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("type" = String, Path, description = "Resource type code or id"),
        ("key" = String, Path, description = "The resource's key value"),
    ),
    responses(
        (status = 200, description = "Archived", body = Resource),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(path): Path<(String, String, String)>,
) -> Result<Json<Resource>, Problem> {
    set_archived(&state, &auth, path, true).await
}

/// Unarchive a customer's resource
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/resources/{type}/{key}/unarchive",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("type" = String, Path, description = "Resource type code or id"),
        ("key" = String, Path, description = "The resource's key value"),
    ),
    responses(
        (status = 200, description = "Unarchived", body = Resource),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(path): Path<(String, String, String)>,
) -> Result<Json<Resource>, Problem> {
    set_archived(&state, &auth, path, false).await
}

/// Delete a customer's resource
///
/// Refused while a subscription (not aborted), a quote, a checkout session or an override, current
/// or past, names it: archive it instead.
#[utoipa::path(
    delete,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/resources/{type}/{key}",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("type" = String, Path, description = "Resource type code or id"),
        ("key" = String, Path, description = "The resource's key value"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Something names the resource", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_resource(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, resource_type, key)): Path<(String, String, String)>,
) -> Result<StatusCode, Problem> {
    let (_, resource_type, found) =
        located(&state, &auth, &id_or_alias, &resource_type, &key).await?;
    let resource = found.ok_or_else(|| not_found(&resource_type, &key))?;
    state
        .store
        .delete_resource(auth.tenant_id, resource.id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

crate::v2_list!(ResourceList, Resource);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct ResourceListQuery {
    /// Only resources of this type, by code or id.
    #[serde(rename = "type")]
    pub resource_type: Option<String>,
    /// Archived resources are left out unless asked.
    #[serde(default)]
    pub include_archived: bool,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

/// List a customer's resources
///
/// Its stored resources in creation order, paged by cursor.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/resources",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ResourceListQuery,
    ),
    responses(
        (status = 200, description = "A page of the customer's resources", body = ResourceList),
        (status = 404, description = "The customer or the resource type is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The cursor or limit is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_resources(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<ResourceListQuery>,
) -> Result<Json<ResourceList>, Problem> {
    let paging = PageQuery {
        limit: query.limit,
        after: query.after.clone(),
    };
    let limit = paging.limit()?;
    let cursor = paging.cursor()?;
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let types = state
        .store
        .list_resource_types(auth.tenant_id, true)
        .await?;
    let resource_type_id = match query.resource_type.as_deref() {
        Some(raw) => {
            let raw = raw.trim();
            let id = ids::parse_id::<ResourceTypeId>(raw);
            Some(
                types
                    .iter()
                    .find(|t| Some(t.id) == id || t.code == raw)
                    .ok_or_else(|| Problem::not_found(format!("resource type {raw}")))?
                    .id,
            )
        }
        None => None,
    };
    let rows = state
        .store
        .list_customer_resources_after(
            auth.tenant_id,
            customer.id,
            resource_type_id,
            query.include_archived,
            cursor.map(|Cursor(after)| ResourceId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |resource| resource.id.as_uuid());
    Ok(Json(ResourceList {
        data: paged
            .data
            .into_iter()
            .map(|resource| {
                let resource_type = types
                    .iter()
                    .find(|t| t.id == resource.resource_type_id)
                    .cloned();
                match resource_type {
                    Some(resource_type) => to_api(resource, &resource_type),
                    None => Resource {
                        id: resource.id,
                        resource_type: ResourceTypeRef {
                            id: resource.resource_type_id,
                            code: resource.resource_type_id.to_string(),
                        },
                        key: resource.key_value,
                        name: resource.name,
                        attributes: resource.attributes,
                        archived: resource.archived_at.is_some(),
                    },
                }
            })
            .collect(),
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}
