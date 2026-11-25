//! `/api/v2/credit-units`: the tenant's credit units, denominations usage is priced and credit
//! granted in instead of money (a charge's `credit_unit`, a credit pack's grant), each with its
//! selling price per currency. Addressed by id or by code (upper-case, never a currency code).
use crate::AppState;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{PageQuery, keyset};
use crate::v2::problem::{Problem, store_message};
use crate::v2::refs::{ProductRef, ReferenceIds, Refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{BaseId, CreditUnitId, ProductId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::features::ensure_credit_pricing_allowed;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::credit_units::{
    CREDIT_UNIT_MAX_PRECISION, CreditUnit as DomainUnit, CreditUnitCode, CreditUnitNew,
    CreditUnitPatch, CreditUnitPrice, credit_unit_price_in,
};
use meteroid_store::repositories::CreditUnitsInterface;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CreditUnit {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CreditUnitId,
    /// 4 to 16 characters A-Z, 0-9 or `_`, starting with a letter; immutable.
    pub code: String,
    /// "Credit".
    pub name: String,
    /// "Credits".
    pub plural_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// Decimal places of an amount in the unit, 0 to 6; immutable.
    pub precision: u8,
    /// The product its sales are booked under (tax category, accounting code); none: the unit is
    /// not sold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    /// The price of one unit per currency, in its major unit.
    #[schema(schema_with = unit_prices)]
    pub prices: BTreeMap<String, Decimal>,
    /// No new price, pack or grant may use it; what uses it keeps working.
    pub archived: bool,
    pub created_at: ApiDateTime,
    pub updated_at: ApiDateTime,
}

crate::v2_list!(CreditUnitList, CreditUnit);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct CreditUnitListQuery {
    /// Archived units too.
    #[serde(default)]
    pub include_archived: bool,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateCreditUnit {
    /// 4 to 16 characters A-Z, 0-9 or `_`, starting with a letter (upper-cased); immutable.
    pub code: String,
    pub name: String,
    pub plural_name: String,
    #[serde(default)]
    pub symbol: Option<String>,
    /// 0 to 6; immutable.
    pub precision: u8,
    /// The product its sales are booked under, by code or id.
    #[serde(default)]
    pub product: Option<String>,
    /// The price of one unit per currency (`{"EUR": "0.01"}`).
    #[serde(default)]
    #[schema(schema_with = unit_prices_sent)]
    pub prices: BTreeMap<String, Decimal>,
}

/// A merge patch: a field left out is unchanged, `null` clears it. In `prices`, a currency set
/// replaces its price (the previous one is kept for what pinned it) and `null` stops selling the
/// unit in that currency; currencies left out keep theirs.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateCreditUnit {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub plural_name: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub symbol: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub product: Option<Option<String>>,
    #[serde(default)]
    #[schema(schema_with = unit_prices_patch)]
    pub prices: Option<BTreeMap<String, Option<Decimal>>>,
}

fn unit_prices() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    crate::v2::charges::decimal_map(
        "The price of one unit per currency, in its major unit.",
        false,
    )
}

fn unit_prices_sent() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    crate::v2::charges::decimal_map(
        "The price of one unit per currency (`{\"EUR\": \"0.01\"}`).",
        false,
    )
}

fn unit_prices_patch() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    crate::v2::charges::decimal_map(
        "The prices to set, by currency; `null` archives a currency's price.",
        true,
    )
}

fn to_api(unit: DomainUnit, prices: &[CreditUnitPrice], refs: &Refs) -> CreditUnit {
    CreditUnit {
        prices: prices
            .iter()
            .filter(|price| price.credit_unit_id == unit.id && price.archived_at.is_none())
            .map(|price| (price.currency.clone(), price.unit_price.normalize()))
            .collect(),
        product: unit.product_id.map(|id| refs.product(id)),
        id: unit.id,
        code: unit.code.into_inner(),
        name: unit.name,
        plural_name: unit.plural_name,
        symbol: unit.symbol,
        precision: unit.precision,
        archived: unit.archived_at.is_some(),
        created_at: unit.created_at.into(),
        updated_at: unit.updated_at.into(),
    }
}

async fn render(
    state: &AppState,
    tenant_id: TenantId,
    units: Vec<DomainUnit>,
) -> Result<Vec<CreditUnit>, Problem> {
    let prices = state
        .store
        .list_credit_unit_prices(tenant_id, units.iter().map(|unit| unit.id).collect(), false)
        .await?;
    let ids = ReferenceIds {
        products: units.iter().filter_map(|unit| unit.product_id).collect(),
        ..ReferenceIds::default()
    };
    let refs = Refs::load(&state.store, tenant_id, &ids).await?;
    Ok(units
        .into_iter()
        .map(|unit| to_api(unit, &prices, &refs))
        .collect())
}

async fn one(
    state: &AppState,
    tenant_id: TenantId,
    unit: DomainUnit,
) -> Result<CreditUnit, Problem> {
    render(state, tenant_id, vec![unit])
        .await?
        .pop()
        .ok_or_else(Problem::internal)
}

/// The tenant's unit of an id or a code.
pub async fn credit_unit(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<DomainUnit, Problem> {
    let raw = raw.trim();
    let id = CreditUnitId::parse_uuid(raw).ok().or_else(|| {
        raw.starts_with(CreditUnitId::PREFIX)
            .then(|| CreditUnitId::parse_base62(raw).ok())
            .flatten()
    });
    Ok(match id {
        Some(id) => state.store.get_credit_unit(tenant_id, id).await?,
        None => state.store.find_credit_unit_by_code(tenant_id, raw).await?,
    })
}

async fn product_of(
    state: &AppState,
    tenant_id: TenantId,
    product: &str,
) -> Result<ProductId, Problem> {
    ids::product_id(state, tenant_id, product)
        .await
        .map_err(|problem| {
            if problem.status == 404 {
                Problem::field(
                    "/product",
                    "not_found",
                    format!("product {product} not found"),
                )
            } else {
                problem
            }
        })
}

fn label(value: &str, path: &str) -> Result<(), Problem> {
    if value.trim().is_empty() {
        return Err(Problem::field(path, "required", "1 to 64 characters"));
    }
    if value.trim().chars().count() > 64 {
        return Err(Problem::field(path, "invalid", "1 to 64 characters"));
    }
    Ok(())
}

/// The prices asked, each checked at its pointer: `None` stops the sales in its currency.
fn checked_prices(
    prices: BTreeMap<String, Option<Decimal>>,
) -> Result<Vec<(String, Option<Decimal>)>, Problem> {
    prices
        .into_iter()
        .map(|(currency, price)| match price {
            Some(unit_price) => credit_unit_price_in(&currency, unit_price)
                .map(|(code, unit_price)| (code, Some(unit_price)))
                .map_err(|error| Problem::at(format!("/prices/{currency}"), error)),
            None => Ok((currency, None)),
        })
        .collect()
}

/// The unit written, read again with its prices. When it is not read back, as the write
/// returned it, its product by id, its prices as sent (`sent`) or left out when unknown (a write
/// committed never answers an error).
async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    unit: DomainUnit,
    sent: Option<BTreeMap<String, Decimal>>,
) -> Response {
    let mut known = to_api(unit.clone(), &[], &Refs::default());
    let read = match state.store.get_credit_unit(tenant_id, unit.id).await {
        Ok(unit) => one(state, tenant_id, unit).await,
        Err(error) => Err(error.into()),
    };
    committed_write(status, read, || {
        let unknown = sent.is_none();
        known.prices = sent.unwrap_or_default();
        let mut value = serde_json::to_value(known).unwrap_or_default();
        if unknown && let Some(object) = value.as_object_mut() {
            object.remove("prices");
        }
        value
    })
}

async fn gate(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    introduces: bool,
) -> Result<(), Problem> {
    ensure_credit_pricing_allowed(
        state.feature_resolver.as_ref(),
        auth.organization_id,
        introduces,
    )
    .await
    .map_err(Problem::gated)
}

async fn set_archived(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id_or_code: &str,
    archived: bool,
) -> Result<Response, Problem> {
    let unit = credit_unit(state, auth.tenant_id, id_or_code).await?;
    if !archived {
        gate(state, auth, true).await?;
    }
    let patch = CreditUnitPatch::new(auth.tenant_id, unit.id, None, None, None, Some(archived))?;
    let updated = state.store.update_credit_unit(patch).await?;
    Ok(written(state, auth.tenant_id, StatusCode::OK, updated, None).await)
}

/// List credit units
#[utoipa::path(
    get,
    tag = "Credit units",
    path = "/api/v2/credit-units",
    params(CreditUnitListQuery),
    responses(
        (status = 200, description = "A page of the tenant's credit units, archived ones left out unless asked", body = CreditUnitList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_credit_units(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<CreditUnitListQuery>,
) -> Result<Json<CreditUnitList>, Problem> {
    let page = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page.limit()?;
    let cursor = page.cursor()?;
    let rows = state
        .store
        .list_credit_units_after(
            auth.tenant_id,
            query.include_archived,
            cursor.map(|cursor| CreditUnitId::from(cursor.0)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |unit| unit.id.as_uuid());
    let data = render(&state, auth.tenant_id, paged.data).await?;
    Ok(Json(CreditUnitList {
        data,
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}

/// Get a credit unit
#[utoipa::path(
    get,
    tag = "Credit units",
    path = "/api/v2/credit-units/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Credit unit id or code")),
    responses(
        (status = 200, description = "The credit unit and its prices", body = CreditUnit),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<CreditUnit>, Problem> {
    let unit = credit_unit(&state, auth.tenant_id, &id_or_code).await?;
    Ok(Json(one(&state, auth.tenant_id, unit).await?))
}

/// Create a credit unit
///
/// Price usage in it with a charge's `credit_unit`, grant it with a credit pack, and sell it
/// once it has a `product` and a price. The unit is created with its product and prices at
/// once: a refused price creates nothing.
#[utoipa::path(
    post,
    tag = "Credit units",
    path = "/api/v2/credit-units",
    request_body = CreateCreditUnit,
    responses(
        (status = 201, description = "Created", body = CreditUnit),
        (status = 403, description = "Credit pricing is not in your plan", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateCreditUnit>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let code = CreditUnitCode::new(&body.code).map_err(|error| {
        Problem::field("/code", "invalid", store_message(error.current_context()))
    })?;
    label(&body.name, "/name")?;
    label(&body.plural_name, "/plural_name")?;
    if body.precision > CREDIT_UNIT_MAX_PRECISION {
        return Err(Problem::field(
            "/precision",
            "invalid",
            format!("0 to {CREDIT_UNIT_MAX_PRECISION}"),
        ));
    }
    let product_id = match body.product.as_deref() {
        Some(product) => Some(product_of(&state, tenant_id, product).await?),
        None => None,
    };
    gate(&state, &auth, true).await?;
    let new = CreditUnitNew::new(
        tenant_id,
        code.as_str(),
        &body.name,
        &body.plural_name,
        body.symbol.as_deref(),
        body.precision,
    )?;
    let prices = checked_prices(
        body.prices
            .into_iter()
            .map(|(currency, price)| (currency, Some(price)))
            .collect(),
    )?;
    let sent: BTreeMap<String, Decimal> = prices
        .iter()
        .filter_map(|(currency, price)| price.map(|price| (currency.clone(), price)))
        .collect();
    // The unit, its product and its prices in one transaction: a refusal of any writes none.
    let created = state
        .store
        .create_credit_unit_with_prices(new, product_id, sent.clone().into_iter().collect())
        .await
        .map_err(|error| Problem::from(error).code_taken(code.as_str()))?;
    Ok(written(&state, tenant_id, StatusCode::CREATED, created, Some(sent)).await)
}

/// Update a credit unit
///
/// A JSON merge patch; the code and the precision never change.
#[utoipa::path(
    patch,
    tag = "Credit units",
    path = "/api/v2/credit-units/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Credit unit id or code")),
    request_body(content = UpdateCreditUnit, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = CreditUnit),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The unit or its product is archived", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateCreditUnit>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let unit = credit_unit(&state, tenant_id, &id_or_code).await?;
    if let Some(name) = &body.name {
        label(name, "/name")?;
    }
    if let Some(plural_name) = &body.plural_name {
        label(plural_name, "/plural_name")?;
    }
    let product = match &body.product {
        None => None,
        Some(None) => Some(None),
        Some(Some(product)) => Some(Some(product_of(&state, tenant_id, product).await?)),
    };
    let changes = body.name.is_some()
        || body.plural_name.is_some()
        || body.symbol.is_some()
        || product.is_some();
    let patch = if changes {
        let mut patch = CreditUnitPatch::new(
            tenant_id,
            unit.id,
            body.name,
            body.plural_name,
            body.symbol,
            None,
        )?;
        if let Some(product) = product {
            patch = patch.with_product_id(product);
        }
        Some(patch)
    } else {
        None
    };
    let prices = checked_prices(body.prices.unwrap_or_default())?;
    // Its fields and its prices in one transaction: a refusal of any writes none.
    let updated = state
        .store
        .update_credit_unit_with_prices(tenant_id, unit.id, patch, prices)
        .await
        .map_err(|error| Problem::at("/product", error))?;
    Ok(written(&state, tenant_id, StatusCode::OK, updated, None).await)
}

/// Archive a credit unit
///
/// No new price, pack or grant may use it; prices, packs and wallets in it keep working.
#[utoipa::path(
    post,
    tag = "Credit units",
    path = "/api/v2/credit-units/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Credit unit id or code")),
    responses(
        (status = 200, description = "Archived", body = CreditUnit),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    set_archived(&state, &auth, &id_or_code, true).await
}

/// Unarchive a credit unit
#[utoipa::path(
    post,
    tag = "Credit units",
    path = "/api/v2/credit-units/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Credit unit id or code")),
    responses(
        (status = 200, description = "Unarchived", body = CreditUnit),
        (status = 403, description = "Credit pricing is not in your plan", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_credit_unit(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    set_archived(&state, &auth, &id_or_code, false).await
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_credit_units, create_credit_unit))
        .routes(routes!(get_credit_unit, update_credit_unit))
        .routes(routes!(archive_credit_unit))
        .routes(routes!(unarchive_credit_unit))
}
