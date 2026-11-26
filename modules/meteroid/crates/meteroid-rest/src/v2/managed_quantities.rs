//! `/api/v2/managed-quantities`: counts a subscription holds and changes over time (seats,
//! licences), shared by every charge naming them. An archived quantity takes no new charge; the
//! subscriptions holding it keep counting and changing it.
use crate::AppState;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::{Problem, store_message};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{BaseId, ManagedQuantityId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::managed_quantities::{
    ManagedQuantity as DomainQuantity, ManagedQuantityNew, ManagedQuantityPatch, QuantityDecrease,
    QuantityIncrease, validate_quantity_code,
};
use meteroid_store::repositories::ManagedQuantitiesInterface;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// When an increase of a subscription's count takes effect: `immediately` (the default), billed
/// as the change's collection mode says; `next_period`, from the next renewal, nothing billed
/// now.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnIncrease {
    /// At once, billed as the change's collection mode says.
    #[default]
    Immediately,
    /// From the next renewal, nothing billed now.
    NextPeriod,
}

/// When a decrease of a subscription's count takes effect: `at_period_end` (the default), the
/// count billed until the next renewal; `immediately`, the unused time given back.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnDecrease {
    /// Billed until the next renewal.
    #[default]
    AtPeriodEnd,
    /// At once, the unused time given back.
    Immediately,
}

/// A count a subscription holds and changes over time (seats, licences), shared by every charge
/// naming it: a seat change changes it once for all of them.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ManagedQuantity {
    /// The managed quantity's id.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: ManagedQuantityId,
    /// Unique per tenant, immutable; charges and seat changes name it by it.
    pub code: String,
    /// What a buyer reads ("Seats").
    pub name: String,
    /// When an increase takes effect, for every subscription counting it.
    pub on_increase: OnIncrease,
    /// When a decrease takes effect, for every subscription counting it.
    pub on_decrease: OnDecrease,
    /// Takes no new charge; the subscriptions holding it keep counting and changing it.
    pub archived: bool,
    /// When the quantity was created.
    pub created_at: ApiDateTime,
}

crate::v2_list!(ManagedQuantityList, ManagedQuantity);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct ManagedQuantityListQuery {
    /// Archived quantities too.
    #[serde(default)]
    pub include_archived: bool,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

/// A managed quantity to create.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateManagedQuantity {
    /// Lowercase letters, digits, `_` and `-`, 64 at most; unique per tenant, immutable.
    pub code: String,
    /// 1 to 255 characters.
    pub name: String,
    /// `immediately` when left out.
    #[serde(default)]
    pub on_increase: OnIncrease,
    /// `at_period_end` when left out.
    #[serde(default)]
    pub on_decrease: OnDecrease,
}

/// A merge patch; the code never changes.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateManagedQuantity {
    /// 1 to 255 characters.
    #[serde(default)]
    pub name: Option<String>,
    /// Applies to the changes made from then on.
    #[serde(default)]
    pub on_increase: Option<OnIncrease>,
    /// Applies to the changes made from then on.
    #[serde(default)]
    pub on_decrease: Option<OnDecrease>,
}

fn increase(on: OnIncrease) -> QuantityIncrease {
    match on {
        OnIncrease::Immediately => QuantityIncrease::Immediately,
        OnIncrease::NextPeriod => QuantityIncrease::NextPeriod,
    }
}

fn decrease(on: OnDecrease) -> QuantityDecrease {
    match on {
        OnDecrease::AtPeriodEnd => QuantityDecrease::AtPeriodEnd,
        OnDecrease::Immediately => QuantityDecrease::Immediately,
    }
}

fn to_api(quantity: DomainQuantity) -> ManagedQuantity {
    ManagedQuantity {
        id: quantity.id,
        code: quantity.code,
        name: quantity.name,
        on_increase: match quantity.on_increase {
            QuantityIncrease::Immediately => OnIncrease::Immediately,
            QuantityIncrease::NextPeriod => OnIncrease::NextPeriod,
        },
        on_decrease: match quantity.on_decrease {
            QuantityDecrease::AtPeriodEnd => OnDecrease::AtPeriodEnd,
            QuantityDecrease::Immediately => OnDecrease::Immediately,
        },
        archived: quantity.archived_at.is_some(),
        created_at: quantity.created_at.into(),
    }
}

/// List managed quantities
///
/// The tenant's managed quantities in creation order, paged by cursor. Archived ones are left
/// out unless `include_archived`.
#[utoipa::path(
    get,
    tag = "Managed quantities",
    path = "/api/v2/managed-quantities",
    params(ManagedQuantityListQuery),
    responses(
        (status = 200, description = "A page of the tenant's managed quantities, archived ones left out unless asked", body = ManagedQuantityList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_managed_quantities(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<ManagedQuantityListQuery>,
) -> Result<Json<ManagedQuantityList>, Problem> {
    let page_query = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page_query.limit()?;
    let rows = state
        .store
        .list_managed_quantities_after(
            auth.tenant_id,
            query.include_archived,
            page_query
                .cursor()?
                .map(|Cursor(after)| ManagedQuantityId::from(after)),
            limit + 1,
        )
        .await?;
    Ok(Json(
        keyset(rows, limit, |quantity| quantity.id.as_uuid())
            .map(to_api)
            .into(),
    ))
}

/// Get a managed quantity
///
/// By id or code, archived or not.
#[utoipa::path(
    get,
    tag = "Managed quantities",
    path = "/api/v2/managed-quantities/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Managed quantity id or code")),
    responses(
        (status = 200, description = "The managed quantity", body = ManagedQuantity),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_managed_quantity(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ManagedQuantity>, Problem> {
    let quantity = ids::quantity(&state, auth.tenant_id, &id_or_code).await?;
    Ok(Json(to_api(quantity)))
}

/// Create a managed quantity
///
/// A count charges can bill per unit of (seats), its code chosen: charges and subscriptions name
/// it by it. Each subscription sets its own starting count.
#[utoipa::path(
    post,
    tag = "Managed quantities",
    path = "/api/v2/managed-quantities",
    request_body = CreateManagedQuantity,
    responses(
        (status = 201, description = "Created", body = ManagedQuantity),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_managed_quantity(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateManagedQuantity>,
) -> Result<(StatusCode, Json<ManagedQuantity>), Problem> {
    let code = body.code.trim().to_owned();
    validate_quantity_code(&code)
        .map_err(|error| Problem::field("/code", "invalid", store_message(&error)))?;
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field("/name", "required", "A quantity has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    let quantity = state
        .store
        .create_managed_quantity(ManagedQuantityNew {
            tenant_id: auth.tenant_id,
            code: code.clone(),
            name,
            on_increase: increase(body.on_increase),
            on_decrease: decrease(body.on_decrease),
        })
        .await
        .map_err(|error| Problem::at("/code", error).code_taken(&code))?;
    Ok((StatusCode::CREATED, Json(to_api(quantity))))
}

/// Update a managed quantity
///
/// A JSON merge patch of its name and when changes take effect; the code never changes. A new
/// `on_increase` or `on_decrease` applies to every subscription counting it, from its next
/// change.
#[utoipa::path(
    patch,
    tag = "Managed quantities",
    path = "/api/v2/managed-quantities/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Managed quantity id or code")),
    request_body(content = UpdateManagedQuantity, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = ManagedQuantity),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_managed_quantity(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateManagedQuantity>,
) -> Result<Json<ManagedQuantity>, Problem> {
    let quantity = ids::quantity(&state, auth.tenant_id, &id_or_code).await?;
    if body
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(Problem::field("/name", "invalid", "A quantity has a name"));
    }
    if let Some(name) = body.name.as_deref() {
        crate::v2::bounds::name(name.trim(), "/name")?;
    }
    let updated = state
        .store
        .update_managed_quantity(ManagedQuantityPatch {
            tenant_id: auth.tenant_id,
            id: quantity.id,
            name: body.name.map(|name| name.trim().to_owned()),
            on_increase: body.on_increase.map(increase),
            on_decrease: body.on_decrease.map(decrease),
        })
        .await?;
    Ok(Json(to_api(updated)))
}

/// Archive a managed quantity
///
/// Off the catalog for new charges (a plan draft, an add-on or a subscription's own charge
/// naming it is refused); the plans and subscriptions counting it keep billing, and a
/// subscription's count of it can still change.
#[utoipa::path(
    post,
    tag = "Managed quantities",
    path = "/api/v2/managed-quantities/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Managed quantity id or code")),
    responses(
        (status = 200, description = "Archived", body = ManagedQuantity),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_managed_quantity(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ManagedQuantity>, Problem> {
    let quantity = ids::quantity(&state, auth.tenant_id, &id_or_code).await?;
    let archived = state
        .store
        .archive_managed_quantity(auth.tenant_id, quantity.id)
        .await?;
    Ok(Json(to_api(archived)))
}

/// Unarchive a managed quantity
///
/// New charges may count it again. Unarchiving a live quantity changes nothing.
#[utoipa::path(
    post,
    tag = "Managed quantities",
    path = "/api/v2/managed-quantities/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Managed quantity id or code")),
    responses(
        (status = 200, description = "Unarchived", body = ManagedQuantity),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_managed_quantity(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<ManagedQuantity>, Problem> {
    let quantity = ids::quantity(&state, auth.tenant_id, &id_or_code).await?;
    let restored = state
        .store
        .unarchive_managed_quantity(auth.tenant_id, quantity.id)
        .await?;
    Ok(Json(to_api(restored)))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_managed_quantities, create_managed_quantity))
        .routes(routes!(get_managed_quantity, update_managed_quantity))
        .routes(routes!(archive_managed_quantity))
        .routes(routes!(unarchive_managed_quantity))
}
