//! `/api/v2/add-ons`: a charge sold beside a plan, like a plan's, priced per currency and
//! cadence, without options.
use crate::AppState;
use crate::v2::Reader;
use crate::v2::charges::{
    Charge, ChargePrice, EVERY_OPTION, OptionSet, StoredCharge, StoredPrice, render, to_domain,
};
use crate::v2::entitlements::{Entitlement, EntitlementInput};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::Problem;
use crate::v2::references::{CatalogNames, resolve_refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use common_domain::ids::{AddOnId, BaseId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::add_ons::{AddOn as DomainAddOn, AddOnPatch, AddOnPriceChange};
use meteroid_store::domain::entitlement_fold::Tier;
use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::products::ProductInterface;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// A charge sold beside a plan: one product, priced per currency and cadence, the
/// subscription's cadence picking its price. A plan version offers it (`add_ons`), a
/// subscription holds it.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct AddOn {
    /// The add-on's id.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: AddOnId,
    /// Unique per tenant, immutable; paths and plan versions take it in place of the id.
    pub code: String,
    /// What a buyer reads, on the portal, checkout and invoices.
    pub name: String,
    /// What the add-on is, in the tenant's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Its prices are each in every option (`*`).
    pub charge: Charge,
    /// Customers may add it themselves (portal, checkout).
    pub self_serviceable: bool,
    /// How many a subscription may hold; none: no limit. One charging per seat is held once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_instances: Option<i32>,
    /// Its rows, canonical; each counts per instance held (seats held for one sold per seat).
    pub entitlements: Vec<Entitlement>,
    /// No longer sold; the subscriptions holding it keep it.
    pub archived: bool,
    /// When the add-on was created.
    pub created_at: ApiDateTime,
}

crate::v2_list!(AddOnList, AddOn);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct AddOnListQuery {
    /// Archived add-ons too.
    #[serde(default)]
    pub include_archived: bool,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

/// An add-on to create: a priced charge, with what it grants.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateAddOn {
    /// Unique per tenant, immutable; the name's slug when left out.
    #[serde(default)]
    pub code: Option<String>,
    /// 1 to 255 characters.
    pub name: String,
    /// 4,096 characters at most.
    #[serde(default)]
    pub description: Option<String>,
    /// A priced charge (a quantity or a preset, and a price at least): one price per currency
    /// and cadence, each `*`. How it bills never changes once created.
    pub charge: Charge,
    /// Customers may add it themselves (portal, checkout); `false` when left out.
    #[serde(default)]
    pub self_serviceable: bool,
    /// How many a subscription may hold, 1 or more; none: no limit. One charging per seat is
    /// held once.
    #[serde(default)]
    pub max_instances: Option<i32>,
    /// Its rows; `add` by default when it is bought several times or per seat, else `raise_to`.
    #[serde(default)]
    pub entitlements: Vec<EntitlementInput>,
}

/// A merge patch. `prices` replaces every price of the charge; how it bills never changes.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateAddOn {
    /// 1 to 255 characters; it cannot be cleared.
    #[serde(default)]
    pub name: Option<String>,
    /// 4,096 characters at most; `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Customers may add it themselves (portal, checkout).
    #[serde(default)]
    pub self_serviceable: Option<bool>,
    /// How many a subscription may hold, 1 or more; `null`: no limit.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<i32>)]
    pub max_instances: Option<Option<i32>>,
    /// Replaces every price of the charge: one per currency and cadence, each `*`. Prices
    /// billing the add-on another way than it bills are refused (`not_expressible`).
    #[serde(default)]
    pub prices: Option<Vec<ChargePrice>>,
    /// Replaces its rows: one sent the same keeps its id and history, one changed is succeeded,
    /// one left out is revoked; `[]` revokes them all.
    #[serde(default)]
    pub entitlements: Option<Vec<EntitlementInput>>,
}

fn stored(add_on: &DomainAddOn) -> StoredCharge {
    StoredCharge {
        name: add_on.name.clone(),
        product_id: add_on.product_id,
        structure: add_on.fee_structure.clone(),
        prices: add_on
            .catalog_prices()
            .into_iter()
            .map(|price| StoredPrice {
                cadence: price.cadence,
                currency: price.currency,
                pricing: price.pricing,
                credit_unit: price.credit_unit,
                options: OptionSet::Every,
            })
            .collect(),
    }
}

async fn to_api(
    state: &impl Reader,
    tenant_id: TenantId,
    mut add_ons: Vec<DomainAddOn>,
) -> Result<Vec<AddOn>, Problem> {
    let charges: Vec<StoredCharge> = add_ons.iter().map(stored).collect();
    let names = CatalogNames::load(state, tenant_id, &charges).await?;
    let owners = add_ons
        .iter_mut()
        .map(|add_on| std::mem::take(&mut add_on.entitlements))
        .collect();
    let grants = crate::v2::entitlements::render_each(state, tenant_id, owners).await?;
    add_ons
        .into_iter()
        .zip(charges)
        .zip(grants)
        .map(|((add_on, charge), entitlements)| {
            let mut charge = render(&charge, &[], &names, "/charge")?;
            // The add-on's own name is the charge's.
            charge.name = None;
            Ok(AddOn {
                id: add_on.id,
                code: add_on.code,
                name: add_on.name,
                description: add_on.description,
                charge,
                self_serviceable: add_on.self_serviceable,
                max_instances: add_on.max_instances_per_subscription,
                entitlements,
                archived: add_on.archived_at.is_some(),
                created_at: add_on.created_at.into(),
            })
        })
        .collect()
}

pub(crate) async fn one(
    state: &impl Reader,
    tenant_id: TenantId,
    add_on: DomainAddOn,
) -> Result<AddOn, Problem> {
    to_api(state, tenant_id, vec![add_on])
        .await?
        .pop()
        .ok_or_else(Problem::internal)
}

/// The add-on written, as it reads; by its id, code and name when it is not read back (a write
/// committed never answers an error).
async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    add_on: DomainAddOn,
) -> Response {
    let known = serde_json::json!({
        "id": add_on.id.to_string(),
        "code": add_on.code,
        "name": add_on.name,
        "archived": add_on.archived_at.is_some(),
    });
    committed_write(status, one(state, tenant_id, add_on).await, || known)
}

/// The add-on read again once archived or unarchived (the store answers nothing); by its id,
/// code, name and `archived` when it is not read back.
async fn reread(
    state: &AppState,
    tenant_id: TenantId,
    before: DomainAddOn,
    archived: bool,
) -> Response {
    let known = serde_json::json!({
        "id": before.id.to_string(),
        "code": before.code,
        "name": before.name,
        "archived": archived,
    });
    let read = match state.store.get_add_on_by_id(tenant_id, before.id).await {
        Ok(add_on) => one(state, tenant_id, add_on).await,
        Err(error) => Err(error.into()),
    };
    committed_write(StatusCode::OK, read, || known)
}

/// An add-on's prices: each in every option, there being none.
fn add_on_prices(
    prices: Vec<(PriceInput, String)>,
    path: &str,
) -> Result<Vec<PriceInput>, Problem> {
    prices
        .into_iter()
        .enumerate()
        .map(|(index, (price, option))| {
            if option == EVERY_OPTION {
                Ok(price)
            } else {
                Err(Problem::not_expressible(
                    format!("{path}/prices/{index}/option"),
                    "an add-on has no options: its prices are `*`",
                ))
            }
        })
        .collect()
}

/// List add-ons
///
/// The tenant's add-ons in creation order, paged by cursor, each with its charge and what it
/// grants. Archived ones are left out unless `include_archived`.
#[utoipa::path(
    get,
    tag = "Add-ons",
    path = "/api/v2/add-ons",
    params(AddOnListQuery),
    responses(
        (status = 200, description = "A page of the tenant's add-ons, archived ones left out unless asked", body = AddOnList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_add_ons(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<AddOnListQuery>,
) -> Result<Json<AddOnList>, Problem> {
    let page_query = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page_query.limit()?;
    let rows = state
        .store
        .list_add_ons_after(
            auth.tenant_id,
            query.include_archived,
            page_query
                .cursor()?
                .map(|Cursor(after)| AddOnId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |add_on| add_on.id.as_uuid());
    let data = to_api(&state, auth.tenant_id, paged.data).await?;
    Ok(Json(AddOnList {
        data,
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}

/// Get an add-on
///
/// By id or code, archived or not, with its charge and what it grants.
#[utoipa::path(
    get,
    tag = "Add-ons",
    path = "/api/v2/add-ons/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Add-on id or code")),
    responses(
        (status = 200, description = "The add-on", body = AddOn),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_add_on(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<AddOn>, Problem> {
    let id = ids::add_on_id(&state, auth.tenant_id, &id_or_code).await?;
    let add_on = state.store.get_add_on_by_id(auth.tenant_id, id).await?;
    Ok(Json(one(&state, auth.tenant_id, add_on).await?))
}

/// Create an add-on
///
/// A priced charge of an existing product, sold beside plans whose versions offer it. Its
/// prices are per currency and cadence, each `*` (an add-on has no options); a charge without a
/// quantity, a preset or a price is refused.
#[utoipa::path(
    post,
    tag = "Add-ons",
    path = "/api/v2/add-ons",
    request_body = CreateAddOn,
    responses(
        (status = 201, description = "Created", body = AddOn),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused, or the model cannot express the charge", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_add_on(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateAddOn>,
) -> Result<Response, Problem> {
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field("/name", "required", "An add-on has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    crate::v2::bounds::description(body.description.as_deref(), "/description")?;
    if body.max_instances.is_some_and(|max| max < 1) {
        return Err(Problem::field(
            "/max_instances",
            "invalid",
            "An add-on is held once at least",
        ));
    }
    let refs = resolve_refs(&state, auth.tenant_id, &body.charge, "/charge").await?;
    let charge = to_domain(&body.charge, &refs, "/charge")?;
    let Some(structure) = charge.structure else {
        return Err(Problem::field(
            "/charge",
            "required",
            "An add-on is priced: send its quantity or a preset, and its prices",
        ));
    };
    let prices = add_on_prices(charge.prices, "/charge")?;
    if prices.is_empty() {
        return Err(Problem::field(
            "/charge/prices",
            "required",
            "An add-on has a price at least",
        ));
    }
    let product = state
        .store
        .find_product_by_id(charge.product_id, auth.tenant_id)
        .await?;
    ids::check_code(body.code.as_deref(), AddOnId::PREFIX)?;
    let entitlements = crate::v2::entitlements::specs(
        &state,
        auth.tenant_id,
        &body.entitlements,
        Tier::AddOn,
        "/entitlements",
    )
    .await?;
    let code = body.code.clone().unwrap_or_default();
    let add_on = state
        .store
        .create_add_on_with_code(
            name,
            body.code,
            ProductRef::Existing(charge.product_id),
            Some(structure),
            prices.into_iter().map(PriceEntry::New).collect(),
            body.description,
            body.self_serviceable,
            body.max_instances,
            auth.tenant_id,
            product.product_family_id,
            entitlements,
        )
        .await
        // A store refusal points at the charge only when no row could be refused.
        .map_err(|error| {
            let problem = if body.entitlements.is_empty() {
                Problem::at("/charge", error)
            } else {
                Problem::from(error)
            };
            problem.code_taken(&code)
        })?;
    Ok(written(&state, auth.tenant_id, StatusCode::CREATED, add_on).await)
}

/// Update an add-on
///
/// A JSON merge patch of its live fields; `prices` replaces every price of its charge. How the
/// charge bills its product is fixed once created.
#[utoipa::path(
    patch,
    tag = "Add-ons",
    path = "/api/v2/add-ons/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Add-on id or code")),
    request_body(content = UpdateAddOn, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = AddOn),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_add_on(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateAddOn>,
) -> Result<Response, Problem> {
    let id = ids::add_on_id(&state, auth.tenant_id, &id_or_code).await?;
    if body
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(Problem::field("/name", "invalid", "An add-on has a name"));
    }
    if let Some(name) = body.name.as_deref() {
        crate::v2::bounds::name(name.trim(), "/name")?;
    }
    crate::v2::bounds::description(
        body.description.as_ref().and_then(|text| text.as_deref()),
        "/description",
    )?;
    if body.max_instances.flatten().is_some_and(|max| max < 1) {
        return Err(Problem::field(
            "/max_instances",
            "invalid",
            "An add-on is held once at least",
        ));
    }
    let entitlements = match &body.entitlements {
        None => None,
        Some(inputs) => Some(
            crate::v2::entitlements::specs(
                &state,
                auth.tenant_id,
                inputs,
                Tier::AddOn,
                "/entitlements",
            )
            .await?,
        ),
    };
    let prices = match body.prices {
        None => None,
        Some(prices) => {
            let current = state.store.get_add_on_by_id(auth.tenant_id, id).await?;
            let names = CatalogNames::load(&state, auth.tenant_id, [&stored(&current)]).await?;
            // The charge as it bills, with the prices asked: its structure stays.
            let mut charge = render(&stored(&current), &[], &names, "")?;
            charge.preset = None;
            charge.prices = prices;
            let refs = resolve_refs(&state, auth.tenant_id, &charge, "").await?;
            let repriced = to_domain(&charge, &refs, "")?;
            let same = match (&repriced.structure, &current.fee_structure) {
                (Some(asked), Some(stored)) => asked.same_shape(stored),
                _ => false,
            };
            if !same {
                return Err(Problem::not_expressible(
                    "/prices",
                    "these prices bill the add-on differently than it does: how an add-on \
                     bills is fixed once created",
                ));
            }
            Some(AddOnPriceChange::All(add_on_prices(repriced.prices, "")?))
        }
    };
    let only_rows = entitlements.is_some() && prices.is_none() && body.max_instances.is_none();
    // Its terms, prices and rows in one transaction: a refusal of any writes none.
    let add_on = state
        .store
        .update_add_on_with_entitlements(
            auth.as_actor(),
            AddOnPatch {
                id,
                tenant_id: auth.tenant_id,
                name: body.name.map(|name| name.trim().to_owned()),
                description: body.description,
                self_serviceable: body.self_serviceable,
                max_instances_per_subscription: body.max_instances,
            },
            prices,
            entitlements,
        )
        .await
        .map_err(|error| {
            if only_rows {
                Problem::at("/entitlements", error)
            } else {
                Problem::from(error)
            }
        })?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, add_on).await)
}

/// Archive an add-on
///
/// No longer sold; the subscriptions holding it keep it.
#[utoipa::path(
    post,
    tag = "Add-ons",
    path = "/api/v2/add-ons/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Add-on id or code")),
    responses(
        (status = 200, description = "Archived", body = AddOn),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The add-on's state refuses it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_add_on(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let id = ids::add_on_id(&state, auth.tenant_id, &id_or_code).await?;
    let current = state.store.get_add_on_by_id(auth.tenant_id, id).await?;
    if current.archived_at.is_some() {
        return Ok(Json(one(&state, auth.tenant_id, current).await?).into_response());
    }
    state
        .store
        .archive_add_on(auth.as_actor(), id, auth.tenant_id)
        .await?;
    Ok(reread(&state, auth.tenant_id, current, true).await)
}

/// Unarchive an add-on
///
/// Sold again. Unarchiving a live add-on changes nothing.
#[utoipa::path(
    post,
    tag = "Add-ons",
    path = "/api/v2/add-ons/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Add-on id or code")),
    responses(
        (status = 200, description = "Unarchived", body = AddOn),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_add_on(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let id = ids::add_on_id(&state, auth.tenant_id, &id_or_code).await?;
    let current = state.store.get_add_on_by_id(auth.tenant_id, id).await?;
    if current.archived_at.is_none() {
        return Ok(Json(one(&state, auth.tenant_id, current).await?).into_response());
    }
    state.store.unarchive_add_on(id, auth.tenant_id).await?;
    Ok(reread(&state, auth.tenant_id, current, false).await)
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_add_ons, create_add_on))
        .routes(routes!(get_add_on, update_add_on))
        .routes(routes!(archive_add_on))
        .routes(routes!(unarchive_add_on))
}
