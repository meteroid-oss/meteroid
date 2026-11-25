//! `/api/v2/products`: what is sold. How a product is billed is each charge's, never the
//! product's: a product has no fee type.
use crate::AppState;
use crate::v2::entitlements::{Grant, grant_value};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::Problem;
use crate::v2::product_families::{family_codes, family_ref};
use crate::v2::refs::ProductFamilyRef;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{BaseId, ProductFamilyId, ProductId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::domain::{Product as DomainProduct, ProductNew};
use meteroid_store::repositories::EntitlementsInterface;
use meteroid_store::repositories::entitlements::FeaturePageFilter;
use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Product {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ProductId,
    pub code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub family: ProductFamilyRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// What it grants: the defaults of its features, by feature code.
    pub entitlements: Vec<Grant>,
    pub archived: bool,
    pub created_at: ApiDateTime,
}

crate::v2_list!(ProductList, Product);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct ProductListQuery {
    /// Only the products of this family, by code or id.
    pub family: Option<String>,
    /// Archived products too.
    #[serde(default)]
    pub include_archived: bool,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateProduct {
    /// Unique per tenant, immutable; the name's slug when left out.
    #[serde(default)]
    pub code: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The product family, by code or id; the default family when left out.
    #[serde(default)]
    pub family: Option<String>,
    /// A lucide icon name the dashboard ships (`box`, `cpu`, `users`…).
    #[serde(default)]
    pub icon: Option<String>,
    /// A color token (`slate`, `blue`, `green`, `amber`, `red`, `violet`, `pink`, `teal`).
    #[serde(default)]
    pub color: Option<String>,
}

/// A merge patch: a field left out is unchanged, `null` clears it.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateProduct {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub icon: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub color: Option<Option<String>>,
    /// Sets the default of each of its features named; one of its features left out loses its
    /// default. Features are made with their product (`POST /features`).
    #[serde(default)]
    pub entitlements: Option<Vec<Grant>>,
}

/// What each product grants: the defaults of its live features, all read once.
pub async fn product_grants(
    state: &AppState,
    tenant_id: TenantId,
) -> Result<HashMap<ProductId, Vec<Grant>>, Problem> {
    let features = state
        .store
        .list_features_after(
            tenant_id,
            FeaturePageFilter {
                statuses: Some(vec![FeatureStatusEnum::Active, FeatureStatusEnum::Disabled]),
                product_id: None,
            },
            None,
            usize::MAX,
        )
        .await?;
    let mut grants: HashMap<ProductId, Vec<Grant>> = HashMap::new();
    for feature in features {
        let (Some(product), Some(default)) = (feature.product, feature.entitlement) else {
            continue;
        };
        grants.entry(product.id).or_default().push(Grant {
            feature: feature.code,
            value: grant_value(&default.value),
        });
    }
    for list in grants.values_mut() {
        list.sort_by(|a, b| a.feature.cmp(&b.feature));
    }
    Ok(grants)
}

pub fn to_api(
    product: DomainProduct,
    families: &HashMap<ProductFamilyId, String>,
    grants: &HashMap<ProductId, Vec<Grant>>,
) -> Product {
    Product {
        entitlements: grants.get(&product.id).cloned().unwrap_or_default(),
        id: product.id,
        family: family_ref(families, product.product_family_id),
        code: product.code,
        name: product.name,
        description: product.description,
        icon: product.icon,
        color: product.color,
        archived: product.archived_at.is_some(),
        created_at: product.created_at.into(),
    }
}

/// The product written, as it reads. When its family and grants are not read back, what the
/// write returned: its family by id, and its grants as sent (`granted`), or left out when unknown
/// (a write committed never answers an error).
async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    product: DomainProduct,
    granted: Option<Vec<Grant>>,
) -> Response {
    let known = product.clone();
    let read = async {
        let families = family_codes(state, tenant_id).await?;
        let grants = product_grants(state, tenant_id).await?;
        Ok::<_, Problem>(to_api(product, &families, &grants))
    }
    .await;
    committed_write(status, read, || {
        let grants = granted
            .as_ref()
            .map(|granted| HashMap::from([(known.id, granted.clone())]));
        let mut value = serde_json::to_value(to_api(
            known,
            &HashMap::new(),
            &grants.clone().unwrap_or_default(),
        ))
        .unwrap_or_default();
        if grants.is_none()
            && let Some(object) = value.as_object_mut()
        {
            object.remove("entitlements");
        }
        value
    })
}

/// List products
#[utoipa::path(
    get,
    tag = "Products",
    path = "/api/v2/products",
    params(ProductListQuery),
    responses(
        (status = 200, description = "A page of the tenant's products, archived ones left out unless asked", body = ProductList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_products(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<ProductListQuery>,
) -> Result<Json<ProductList>, Problem> {
    let family = match query.family.as_deref() {
        Some(family) => Some(ids::product_family_id(&state, auth.tenant_id, family).await?),
        None => None,
    };
    let page_query = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page_query.limit()?;
    let rows = state
        .store
        .list_products_after(
            auth.tenant_id,
            family,
            query.include_archived,
            page_query
                .cursor()?
                .map(|Cursor(after)| ProductId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |product| product.id.as_uuid());
    let families = family_codes(&state, auth.tenant_id).await?;
    let grants = product_grants(&state, auth.tenant_id).await?;
    Ok(Json(
        paged
            .map(|product| to_api(product, &families, &grants))
            .into(),
    ))
}

/// Get a product
#[utoipa::path(
    get,
    tag = "Products",
    path = "/api/v2/products/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Product id or code")),
    responses(
        (status = 200, description = "The product", body = Product),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_product(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<Product>, Problem> {
    let id = ids::product_id(&state, auth.tenant_id, &id_or_code).await?;
    let product = state.store.find_product_by_id(id, auth.tenant_id).await?;
    let families = family_codes(&state, auth.tenant_id).await?;
    let grants = product_grants(&state, auth.tenant_id).await?;
    Ok(Json(to_api(product, &families, &grants)))
}

/// Create a product
#[utoipa::path(
    post,
    tag = "Products",
    path = "/api/v2/products",
    request_body = CreateProduct,
    responses(
        (status = 201, description = "Created", body = Product),
        (status = 404, description = "The family is not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_product(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateProduct>,
) -> Result<Response, Problem> {
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field("/name", "required", "A product has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    crate::v2::bounds::description(body.description.as_deref(), "/description")?;
    let family_id = crate::v2::product_families::family_for_new_use(
        &state,
        auth.tenant_id,
        body.family.as_deref(),
        "/family",
    )
    .await?;
    ids::check_code(body.code.as_deref(), ProductId::PREFIX)?;
    let code = body.code.clone().unwrap_or_default();
    let product = state
        .store
        .create_product_with_code(
            auth.as_actor(),
            ProductNew {
                name,
                description: body.description,
                tenant_id: auth.tenant_id,
                family_id,
                catalog: true,
                tax_category_id: None,
                icon: body.icon,
                color: body.color,
            },
            body.code,
        )
        .await
        .map_err(|error| Problem::from(error).code_taken(&code))?;
    // A new product grants nothing yet.
    Ok(written(
        &state,
        auth.tenant_id,
        StatusCode::CREATED,
        product,
        Some(Vec::new()),
    )
    .await)
}

/// Update a product
///
/// A JSON merge patch; the code never changes.
#[utoipa::path(
    patch,
    tag = "Products",
    path = "/api/v2/products/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Product id or code")),
    request_body(content = UpdateProduct, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Product),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_product(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateProduct>,
) -> Result<Response, Problem> {
    let id = ids::product_id(&state, auth.tenant_id, &id_or_code).await?;
    if body
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(Problem::field("/name", "invalid", "A product has a name"));
    }
    if let Some(name) = body.name.as_deref() {
        crate::v2::bounds::name(name.trim(), "/name")?;
    }
    crate::v2::bounds::description(
        body.description.as_ref().and_then(|text| text.as_deref()),
        "/description",
    )?;
    // The grants are checked before anything is written (their features, values and that each
    // feature is the product's, again when they are set); an icon or a color too, so a refusal
    // of the write is the grants'.
    let entitlements = body.entitlements.clone();
    let feature_defaults = match &entitlements {
        Some(grants) => Some(
            crate::v2::entitlements::product_specs(
                &state,
                auth.tenant_id,
                id,
                grants,
                "/entitlements",
            )
            .await?,
        ),
        None => None,
    };
    meteroid_store::domain::validate_product_appearance(
        body.icon.as_ref().and_then(|icon| icon.as_deref()),
        body.color.as_ref().and_then(|color| color.as_deref()),
    )
    .map_err(Problem::from)?;
    // Its fields and its grants in one transaction: a refusal of either writes neither.
    let product = state
        .store
        .update_product_with_feature_defaults(
            auth.as_actor(),
            ProductUpdate {
                id,
                tenant_id: auth.tenant_id,
                name: body.name.map(|name| name.trim().to_owned()),
                description: body.description,
                tax_category_id: None,
                icon: body.icon,
                color: body.color,
            },
            feature_defaults,
        )
        .await
        .map_err(|error| match &entitlements {
            Some(_) => Problem::at("/entitlements", error),
            None => Problem::from(error),
        })?;
    Ok(written(
        &state,
        auth.tenant_id,
        StatusCode::OK,
        product,
        entitlements,
    )
    .await)
}

/// Archive a product
///
/// Off the catalog for new charges; the charges billing it keep billing it.
#[utoipa::path(
    post,
    tag = "Products",
    path = "/api/v2/products/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Product id or code")),
    responses(
        (status = 200, description = "Archived", body = Product),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The product's state refuses it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_product(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let id = ids::product_id(&state, auth.tenant_id, &id_or_code).await?;
    let current = state.store.find_product_by_id(id, auth.tenant_id).await?;
    let product = if current.archived_at.is_some() {
        current
    } else {
        state
            .store
            .archive_product(auth.as_actor(), id, auth.tenant_id)
            .await?
    };
    Ok(written(&state, auth.tenant_id, StatusCode::OK, product, None).await)
}

/// Unarchive a product
#[utoipa::path(
    post,
    tag = "Products",
    path = "/api/v2/products/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Product id or code")),
    responses(
        (status = 200, description = "Unarchived", body = Product),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_product(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let id = ids::product_id(&state, auth.tenant_id, &id_or_code).await?;
    let current = state.store.find_product_by_id(id, auth.tenant_id).await?;
    let product = if current.archived_at.is_none() {
        current
    } else {
        state.store.unarchive_product(id, auth.tenant_id).await?
    };
    Ok(written(&state, auth.tenant_id, StatusCode::OK, product, None).await)
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_products, create_product))
        .routes(routes!(get_product, update_product))
        .routes(routes!(archive_product))
        .routes(routes!(unarchive_product))
}
