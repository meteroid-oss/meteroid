//! `/api/v2/product-families`: the product lines a catalog is sorted in, each with its pricing
//! page. An archived family takes no new product, plan or metric; what is in it keeps selling.
use crate::AppState;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::Problem;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{BaseId, ProductFamilyId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::{PaginationRequest, ProductFamily as DomainFamily, ProductFamilyNew};
use meteroid_store::repositories::ProductFamilyInterface;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// The whole of a tenant's catalog collection, paged by cursor in the API (`pagination`).
pub const WHOLE: PaginationRequest = PaginationRequest {
    per_page: Some(10_000),
    page: 0,
};

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ProductFamily {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ProductFamilyId,
    pub code: String,
    pub name: String,
    pub archived: bool,
    pub created_at: ApiDateTime,
}

crate::v2_list!(ProductFamilyList, ProductFamily);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct ProductFamilyListQuery {
    /// Archived families too.
    #[serde(default)]
    pub include_archived: bool,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateProductFamily {
    /// Unique per tenant, immutable; the name's slug when left out.
    #[serde(default)]
    pub code: Option<String>,
    pub name: String,
}

/// A merge patch.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductFamily {
    #[serde(default)]
    pub name: Option<String>,
}

fn to_api(family: DomainFamily) -> ProductFamily {
    ProductFamily {
        id: family.id,
        code: family.code,
        name: family.name,
        archived: family.archived_at.is_some(),
        created_at: family.created_at.into(),
    }
}

/// The family a new product or plan goes in, read at `pointer`: the one named, live, else the
/// tenant's first live family.
pub async fn family_for_new_use(
    state: &AppState,
    tenant_id: TenantId,
    named: Option<&str>,
    pointer: &str,
) -> Result<ProductFamilyId, Problem> {
    let Some(raw) = named else {
        return Ok(state.store.find_default_product_family(tenant_id).await?.id);
    };
    let id = ids::product_family_id(state, tenant_id, raw)
        .await
        .map_err(|problem| {
            if problem.status == 404 {
                Problem::field(pointer, "not_found", format!("family {raw} not found"))
            } else {
                problem
            }
        })?;
    let family = state.store.find_product_family_by_id(id, tenant_id).await?;
    if family.archived_at.is_some() {
        return Err(Problem::field(
            pointer,
            "archived",
            format!("family {} is archived", family.code),
        ));
    }
    Ok(id)
}

/// The tenant's families by id, with their codes.
/// The family of `id`, by the code `families` holds of it.
pub fn family_ref(
    families: &HashMap<ProductFamilyId, String>,
    id: ProductFamilyId,
) -> crate::v2::refs::ProductFamilyRef {
    crate::v2::refs::ProductFamilyRef {
        id,
        code: families.get(&id).cloned().unwrap_or_else(|| id.to_string()),
    }
}

pub async fn family_codes(
    state: &AppState,
    tenant_id: TenantId,
) -> Result<HashMap<ProductFamilyId, String>, Problem> {
    Ok(state
        .store
        .list_product_families(tenant_id, WHOLE, None, None)
        .await?
        .items
        .into_iter()
        .map(|family| (family.id, family.code))
        .collect())
}

/// List product families
#[utoipa::path(
    get,
    tag = "Product families",
    path = "/api/v2/product-families",
    params(ProductFamilyListQuery),
    responses(
        (status = 200, description = "A page of the tenant's product families, archived ones left out unless asked", body = ProductFamilyList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_product_families(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<ProductFamilyListQuery>,
) -> Result<Json<ProductFamilyList>, Problem> {
    let page_query = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page_query.limit()?;
    let rows = state
        .store
        .list_product_families_after(
            auth.tenant_id,
            query.include_archived,
            page_query
                .cursor()?
                .map(|Cursor(after)| ProductFamilyId::from(after)),
            limit + 1,
        )
        .await?;
    Ok(Json(
        keyset(rows, limit, |family| family.id.as_uuid())
            .map(to_api)
            .into(),
    ))
}

/// Get a product family
#[utoipa::path(
    get,
    tag = "Product families",
    path = "/api/v2/product-families/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Product family id or code")),
    responses(
        (status = 200, description = "The product family", body = ProductFamily),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_product_family(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ProductFamily>, Problem> {
    let id = ids::product_family_id(&state, auth.tenant_id, &id_or_code).await?;
    let family = state
        .store
        .find_product_family_by_id(id, auth.tenant_id)
        .await?;
    Ok(Json(to_api(family)))
}

/// Create a product family
#[utoipa::path(
    post,
    tag = "Product families",
    path = "/api/v2/product-families",
    request_body = CreateProductFamily,
    responses(
        (status = 201, description = "Created", body = ProductFamily),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_product_family(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateProductFamily>,
) -> Result<(StatusCode, Json<ProductFamily>), Problem> {
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field(
            "/name",
            "required",
            "A product family has a name",
        ));
    }
    crate::v2::bounds::name(&name, "/name")?;
    ids::check_code(body.code.as_deref(), ProductFamilyId::PREFIX)?;
    let code = body.code.clone();
    let family = state
        .store
        .insert_product_family_with_code(
            ProductFamilyNew {
                name,
                tenant_id: auth.tenant_id,
            },
            body.code,
            Some(auth.as_actor()),
        )
        .await
        .map_err(|error| Problem::from(error).code_taken(code.as_deref().unwrap_or("")))?;
    Ok((StatusCode::CREATED, Json(to_api(family))))
}

/// Update a product family
///
/// A JSON merge patch of its name; the code never changes.
#[utoipa::path(
    patch,
    tag = "Product families",
    path = "/api/v2/product-families/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Product family id or code")),
    request_body(content = UpdateProductFamily, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = ProductFamily),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_product_family(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateProductFamily>,
) -> Result<Json<ProductFamily>, Problem> {
    let id = ids::product_family_id(&state, auth.tenant_id, &id_or_code).await?;
    let family = match body.name.map(|name| name.trim().to_owned()) {
        Some(name) if name.is_empty() => {
            return Err(Problem::field(
                "/name",
                "invalid",
                "A product family has a name",
            ));
        }
        Some(name) => {
            crate::v2::bounds::name(&name, "/name")?;
            state
                .store
                .rename_product_family(id, name, auth.tenant_id)
                .await?
        }
        None => {
            state
                .store
                .find_product_family_by_id(id, auth.tenant_id)
                .await?
        }
    };
    Ok(Json(to_api(family)))
}

/// Archive a product family
///
/// Off the catalog for new products, plans and metrics; what is in it keeps selling and
/// billing, and its pricing page is still read.
#[utoipa::path(
    post,
    tag = "Product families",
    path = "/api/v2/product-families/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Product family id or code")),
    responses(
        (status = 200, description = "Archived", body = ProductFamily),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_product_family(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ProductFamily>, Problem> {
    let id = ids::product_family_id(&state, auth.tenant_id, &id_or_code).await?;
    let family = state
        .store
        .archive_product_family(id, auth.tenant_id)
        .await?;
    Ok(Json(to_api(family)))
}

/// Unarchive a product family
#[utoipa::path(
    post,
    tag = "Product families",
    path = "/api/v2/product-families/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Product family id or code")),
    responses(
        (status = 200, description = "Unarchived", body = ProductFamily),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_product_family(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ProductFamily>, Problem> {
    let id = ids::product_family_id(&state, auth.tenant_id, &id_or_code).await?;
    let family = state
        .store
        .unarchive_product_family(id, auth.tenant_id)
        .await?;
    Ok(Json(to_api(family)))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_product_families, create_product_family))
        .routes(routes!(get_product_family, update_product_family))
        .routes(routes!(archive_product_family))
        .routes(routes!(unarchive_product_family))
}
