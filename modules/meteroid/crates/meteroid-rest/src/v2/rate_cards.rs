//! `/api/v2/rate-cards`: the dated rates of a metric's usage by its dimensions. A card is created
//! with what it rates (fixed once made), renamed, its rates set in batches with v1's logic
//! (R-131), and each change read back from its activity (`…/changes`). An archived card prices
//! no new charge; the prices it rates keep rating and its rates can still be set.
use crate::AppState;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::Problem;
use crate::v2::refs::{CreditUnitRef, MetricRef, ReferenceIds, Refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::NaiveDate;
use common_domain::actor::ActorType;
use common_domain::ids::{BaseId, CreditUnitId, RateCardId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entity_activity::{ActivityType, EntityType};
use meteroid_store::domain::rate_cards as domain;
use meteroid_store::repositories::RateCardsInterface;
use meteroid_store::repositories::entity_activity::{ActivityCursor, EntityActivityInterface};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// What a card's rates are in: `money`, a rate per currency on each key; `credit_unit`, rates in
/// the card's one credit unit.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Denomination {
    /// A rate per currency on each key.
    Money,
    /// Rates in one credit unit.
    CreditUnit,
}

/// A key's rate in a currency from a day on.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Rate {
    /// A pattern per dimension: a value, a prefix ending in `*`, or `*`.
    pub key: Vec<String>,
    /// None on a credit unit card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// None on a scheduled removal of the key.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub rate: Option<Decimal>,
    /// The day it is in force from.
    pub effective_from: NaiveDate,
    /// A free-text name of the key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Dated rates of a metric's usage by its dimensions: each usage event is rated by the key its
/// dimensions' values match. What it rates is fixed once created.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct RateCard {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: RateCardId,
    pub code: String,
    pub name: String,
    /// The metric rated.
    pub metric: MetricRef,
    /// The event properties a rate's `key` matches, in its order.
    pub dimensions: Vec<String>,
    pub denomination: Denomination,
    /// The credit unit the rates are in, on a `credit_unit` card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_unit: Option<CreditUnitRef>,
    /// Rates are per this many units (1000000: per 1M tokens).
    #[schema(value_type = String, format = "decimal")]
    pub unit_size: Decimal,
    /// What usage no key matches does: `hold` keeps the invoice waiting until it is rated,
    /// `ignore` leaves it unbilled.
    pub on_unmatched: crate::v2::charges::Unmatched,
    /// Archived: it prices no new charge; the prices it rates keep rating.
    pub archived: bool,
    pub created_at: ApiDateTime,
    /// The day `rates` are in force on (a single card only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<NaiveDate>,
    /// The rate of each key and currency in force on `at` (a single card only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rates: Option<Vec<Rate>>,
    /// The changes dated after `at` (a single card only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<Vec<Rate>>,
}

crate::v2_list!(RateCardList, RateCard);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct RateCardListQuery {
    /// Only the cards rating this metric, by code or id.
    pub metric: Option<String>,
    /// Archived cards too.
    #[serde(default)]
    pub include_archived: bool,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct RateCardGetQuery {
    /// The day the rates are read on; today when left out.
    pub at: Option<NaiveDate>,
}

/// A key's rate to set from the batch's day.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RateInput {
    /// A pattern per dimension, in the card's order: a value, a prefix ending in `*`, or `*`.
    pub key: Vec<String>,
    /// Required on a money card, left out on a credit unit card.
    #[serde(default)]
    pub currency: Option<String>,
    /// Per `unit_size` units, a decimal string: in the currency's major unit on a money card, in
    /// credit units on a credit unit card.
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
    /// A free-text name of the key.
    #[serde(default)]
    pub label: Option<String>,
}

/// A key to remove from the batch's day: its usage is then rated by the other keys it matches,
/// or is unmatched. Its rates scheduled after that day are withdrawn.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RateRemoval {
    /// The key as its rates were set.
    pub key: Vec<String>,
    /// The currency it is removed in, on a money card; left out on a credit unit card.
    #[serde(default)]
    pub currency: Option<String>,
}

/// Rates set and keys removed together, all from one day (today when left out). Keys not
/// listed keep their rates; a repeated batch changes nothing.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SetRates {
    /// The day the whole batch takes effect; today when left out. Only a key's first rate in a
    /// currency may start on a past day.
    #[serde(default)]
    pub effective_from: Option<NaiveDate>,
    /// Rates to set, 5,000 at most.
    #[serde(default)]
    pub rates: Vec<RateInput>,
    /// Keys to remove, 5,000 at most.
    #[serde(default)]
    pub remove: Vec<RateRemoval>,
}

/// What a batch of rates did, and the card as of its day.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct RatesSet {
    /// Rates added.
    pub inserted: usize,
    /// Keys removed.
    pub removed: usize,
    /// Rates already in force, or already there: left as they were.
    pub unchanged: usize,
    /// Scheduled rates the batch replaced or withdrew.
    pub replaced: usize,
    /// The card as of the batch's day.
    pub rate_card: RateCard,
}

fn today() -> NaiveDate {
    common_clock::now().date_naive()
}

fn rate(entry: &domain::RateCardEntry) -> Rate {
    Rate {
        key: entry.key_text(),
        currency: entry.currency.clone(),
        rate: entry.rate.map(|rate| rate.normalize()),
        effective_from: entry.effective_from,
        label: entry.label.clone(),
    }
}

fn to_api(
    card: domain::RateCard,
    refs: &Refs,
    entries: Option<(&[domain::RateCardEntry], NaiveDate)>,
) -> RateCard {
    let (denomination, credit_unit) = match card.denomination {
        domain::RateCardDenomination::Money => (Denomination::Money, None),
        domain::RateCardDenomination::CreditUnit(unit) => {
            (Denomination::CreditUnit, Some(refs.credit_unit(unit)))
        }
    };
    let (at, rates, scheduled) = match entries {
        None => (None, None, None),
        Some((entries, at)) => {
            let rates = domain::rates_at(entries, at)
                .into_iter()
                .filter(|entry| entry.rate.is_some())
                .map(rate)
                .collect();
            let mut scheduled: Vec<Rate> = entries
                .iter()
                .filter(|entry| entry.effective_from > at)
                .map(rate)
                .collect();
            scheduled.sort_by(|a, b| {
                (a.effective_from, &a.key, &a.currency).cmp(&(
                    b.effective_from,
                    &b.key,
                    &b.currency,
                ))
            });
            (Some(at), Some(rates), Some(scheduled))
        }
    };
    RateCard {
        id: card.id,
        metric: refs.metric(card.metric_id),
        code: card.code,
        name: card.name,
        dimensions: card.dimensions,
        denomination,
        credit_unit,
        unit_size: card.unit_size.normalize(),
        on_unmatched: match card.on_unmatched {
            domain::RateCardUnmatched::Hold => crate::v2::charges::Unmatched::Hold,
            domain::RateCardUnmatched::Ignore => crate::v2::charges::Unmatched::Ignore,
        },
        archived: card.archived_at.is_some(),
        created_at: card.created_at.into(),
        at,
        rates,
        scheduled,
    }
}

/// The metrics and credit units the cards name, read in one round.
async fn card_refs(
    state: &AppState,
    tenant_id: TenantId,
    cards: &[domain::RateCard],
) -> Result<Refs, Problem> {
    let ids = ReferenceIds {
        metrics: cards.iter().map(|card| card.metric_id).collect(),
        credit_units: cards
            .iter()
            .filter_map(|card| match card.denomination {
                domain::RateCardDenomination::CreditUnit(unit) => Some(unit),
                domain::RateCardDenomination::Money => None,
            })
            .collect(),
        ..ReferenceIds::default()
    };
    Refs::load(&state.store, tenant_id, &ids).await
}

/// The card as it reads, with its rates at `at`.
async fn read_card(
    state: &AppState,
    tenant_id: TenantId,
    id: RateCardId,
    at: NaiveDate,
) -> Result<RateCard, Problem> {
    let (card, entries) = state.store.get_rate_card(tenant_id, id).await?;
    let refs = card_refs(state, tenant_id, std::slice::from_ref(&card)).await?;
    Ok(to_api(card, &refs, Some((&entries, at))))
}

/// A card written, read back; by its id when it is not (the write is made all the same: a write
/// committed never answers an error).
async fn written(state: &AppState, tenant_id: TenantId, id: RateCardId) -> Response {
    committed_write(
        StatusCode::OK,
        read_card(state, tenant_id, id, today()).await,
        || serde_json::json!({"id": id.to_string()}),
    )
}

/// List rate cards
///
/// The tenant's cards in creation order, paged by cursor, without their rates (read a card for
/// them); archived ones only with `include_archived`.
#[utoipa::path(
    get,
    tag = "Rate cards",
    path = "/api/v2/rate-cards",
    params(RateCardListQuery),
    responses(
        (status = 200, description = "A page of the tenant's rate cards, archived ones left out unless asked", body = RateCardList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_rate_cards(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<RateCardListQuery>,
) -> Result<Json<RateCardList>, Problem> {
    let metric = match query.metric.as_deref() {
        Some(metric) => Some(ids::metric(&state, auth.tenant_id, metric).await?.id),
        None => None,
    };
    let page_query = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page_query.limit()?;
    let rows = state
        .store
        .list_rate_cards_after(
            auth.tenant_id,
            metric,
            query.include_archived,
            page_query
                .cursor()?
                .map(|Cursor(after)| RateCardId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |card| card.id.as_uuid());
    let refs = card_refs(&state, auth.tenant_id, &paged.data).await?;
    Ok(Json(paged.map(|card| to_api(card, &refs, None)).into()))
}

/// Get a rate card
///
/// The card with the rate of each key and currency in force on `at` (today by default), and
/// the changes scheduled after it.
#[utoipa::path(
    get,
    tag = "Rate cards",
    path = "/api/v2/rate-cards/{id_or_code}",
    params(
        ("id_or_code" = String, Path, description = "Rate card id or code"),
        RateCardGetQuery,
    ),
    responses(
        (status = 200, description = "The rate card and its rates", body = RateCard),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_rate_card(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Query(query): Query<RateCardGetQuery>,
) -> Result<Json<RateCard>, Problem> {
    let id = ids::rate_card_id(&state, auth.tenant_id, &id_or_code).await?;
    let (card, entries) = state.store.get_rate_card(auth.tenant_id, id).await?;
    let refs = card_refs(&state, auth.tenant_id, std::slice::from_ref(&card)).await?;
    let at = query.at.unwrap_or_else(today);
    Ok(Json(to_api(card, &refs, Some((&entries, at)))))
}

/// Set a rate card's rates
///
/// Changes only the keys and currencies listed: sets their rates and removes keys, the whole
/// batch from `effective_from` (today by default). The body lists changes, not a merge patch:
/// it is declared `application/json`, and `application/merge-patch+json` reads alike. Entries are never edited: a change is a new
/// rate from its day, and earlier days keep their rates. A key's first rate in a currency may
/// start on a past day unless another key rated its usage since; a change or a removal starts
/// today or later.
#[utoipa::path(
    patch,
    tag = "Rate cards",
    path = "/api/v2/rate-cards/{id_or_code}/rates",
    operation_id = "update_rate_card_rates",
    params(("id_or_code" = String, Path, description = "Rate card id or code")),
    request_body(content = SetRates, content_type = "application/json"),
    responses(
        (status = 200, description = "What the batch did, and the card as of its day", body = RatesSet),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The batch is refused as a whole", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn set_rates(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<SetRates>,
) -> Result<Response, Problem> {
    crate::v2::bounds::count(
        body.rates.len(),
        crate::v2::bounds::MAX_RATES_PER_WRITE,
        "/rates",
        "rates",
    )?;
    crate::v2::bounds::count(
        body.remove.len(),
        crate::v2::bounds::MAX_RATES_PER_WRITE,
        "/remove",
        "keys removed",
    )?;
    let id = ids::rate_card_id(&state, auth.tenant_id, &id_or_code).await?;
    let today = today();
    let at = body.effective_from.unwrap_or(today);
    let set = domain::RateCardRatesSet {
        effective_from: body.effective_from,
        rates: body
            .rates
            .into_iter()
            .map(|rate| domain::RateCardRateInput {
                key: rate.key,
                currency: rate.currency.map(|c| c.trim().to_ascii_uppercase()),
                rate: rate.rate,
                label: rate.label,
            })
            .collect(),
        remove: body
            .remove
            .into_iter()
            .map(|removal| domain::RateCardKeyRemoval {
                key: removal.key,
                currency: removal.currency.map(|c| c.trim().to_ascii_uppercase()),
            })
            .collect(),
    };
    let summary = state
        .store
        .set_rate_card_rates(auth.as_actor(), auth.tenant_id, id, set, today)
        .await?;
    let (inserted, removed, unchanged, replaced) = (
        summary.inserted,
        summary.removed,
        summary.unchanged,
        summary.replaced,
    );
    let read = read_card(&state, auth.tenant_id, id, at)
        .await
        .map(|rate_card| RatesSet {
            inserted,
            removed,
            unchanged,
            replaced,
            rate_card,
        });
    Ok(committed_write(StatusCode::OK, read, || {
        serde_json::json!({
            "inserted": inserted,
            "removed": removed,
            "unchanged": unchanged,
            "replaced": replaced,
            "rate_card": {"id": id.to_string()},
        })
    }))
}

/// What a card's rates are in: `money` (the default), a rate per currency on each key;
/// `credit_unit`, rates in the card's `credit_unit`.
#[derive(Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DenominationInput {
    #[default]
    Money,
    CreditUnit,
}

/// A card and what it rates, fixed once created; its rates are set with `PATCH …/rates`.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateRateCard {
    /// Unique per tenant, immutable.
    pub code: String,
    pub name: String,
    /// The metric rated, by code or id: one that adds up (a sum or a count).
    pub metric: String,
    /// One to four distinct event properties.
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub denomination: DenominationInput,
    /// The credit unit, by id, of a `credit_unit` card.
    #[serde(default)]
    pub credit_unit: Option<String>,
    /// Rates are per this many units; 1 when left out.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub unit_size: Option<Decimal>,
    /// What usage no key matches does: `hold` (the default) keeps the invoice waiting until it
    /// is rated, `ignore` leaves it unbilled.
    #[serde(default)]
    pub on_unmatched: crate::v2::charges::Unmatched,
}

/// A merge patch of what a card may change: its name and what unmatched usage does.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateRateCard {
    #[serde(default)]
    pub name: Option<String>,
    /// What usage no key matches does from then on.
    #[serde(default)]
    pub on_unmatched: Option<crate::v2::charges::Unmatched>,
}

fn unmatched(on: crate::v2::charges::Unmatched) -> domain::RateCardUnmatched {
    match on {
        crate::v2::charges::Unmatched::Hold => domain::RateCardUnmatched::Hold,
        crate::v2::charges::Unmatched::Ignore => domain::RateCardUnmatched::Ignore,
    }
}

/// Create a rate card
///
/// Answers the card created, without rates: set them with `PATCH …/rates`. What it rates (its
/// metric, dimensions, denomination, credit unit and unit size) never changes. A taken `code` is
/// refused (`409 code_taken`), as is a metric that does not add up.
#[utoipa::path(
    post,
    tag = "Rate cards",
    path = "/api/v2/rate-cards",
    request_body = CreateRateCard,
    responses(
        (status = 201, description = "Created, without rates", body = RateCard),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_rate_card(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateRateCard>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    ids::check_code(Some(body.code.as_str()), RateCardId::PREFIX)?;
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field(
            "/name",
            "required",
            "A rate card has a name",
        ));
    }
    crate::v2::bounds::name(&name, "/name")?;
    let metric = ids::metric(&state, tenant_id, &body.metric)
        .await
        .map_err(|problem| {
            if problem.status == 404 {
                Problem::field(
                    "/metric",
                    "not_found",
                    format!("metric {} not found", body.metric),
                )
            } else {
                problem
            }
        })?;
    let dimensions: Vec<String> = body
        .dimensions
        .iter()
        .map(|dimension| dimension.trim().to_owned())
        .collect();
    if dimensions.is_empty() || dimensions.len() > 4 {
        return Err(Problem::field(
            "/dimensions",
            "invalid",
            "A rate card rates by one to four dimensions",
        ));
    }
    for (index, dimension) in dimensions.iter().enumerate() {
        if dimension.is_empty() {
            return Err(Problem::field(
                format!("/dimensions/{index}"),
                "required",
                "A dimension names an event property",
            ));
        }
        if dimensions[..index].contains(dimension) {
            return Err(Problem::field(
                format!("/dimensions/{index}"),
                "duplicate",
                format!("'{dimension}' is named twice"),
            ));
        }
    }
    let unit_size = body.unit_size.unwrap_or(Decimal::ONE);
    if unit_size <= Decimal::ZERO {
        return Err(Problem::field(
            "/unit_size",
            "invalid",
            "A unit size is positive",
        ));
    }
    let denomination = match (body.denomination, body.credit_unit.as_deref()) {
        (DenominationInput::Money, None) => domain::RateCardDenomination::Money,
        (DenominationInput::Money, Some(_)) => {
            return Err(Problem::field(
                "/credit_unit",
                "invalid",
                "A money card has no credit unit",
            ));
        }
        (DenominationInput::CreditUnit, None) => {
            return Err(Problem::field(
                "/credit_unit",
                "required",
                "A credit unit card names its unit",
            ));
        }
        (DenominationInput::CreditUnit, Some(raw)) => {
            let unit = ids::parse_id::<CreditUnitId>(raw)
                .ok_or_else(|| Problem::field("/credit_unit", "invalid", "Not a credit unit id"))?;
            domain::RateCardDenomination::CreditUnit(unit)
        }
    };
    let code = body.code.clone();
    let card = state
        .store
        .create_rate_card(
            auth.as_actor(),
            domain::RateCardNew {
                tenant_id,
                code: body.code,
                name,
                metric_id: metric.id,
                dimensions,
                denomination,
                unit_size,
                on_unmatched: unmatched(body.on_unmatched),
            },
        )
        .await
        .map_err(|error| {
            let problem = Problem::from(error);
            if problem.status == 404 {
                Problem::field("/credit_unit", "not_found", "The credit unit is not found")
            } else if problem.errors.is_empty() && problem.status == 422 {
                let detail = problem.detail.clone().unwrap_or_default();
                Problem::field("/metric", "invalid", detail)
            } else {
                problem.code_taken(&code)
            }
        })?;
    let no_entries: &[domain::RateCardEntry] = &[];
    let known = card.clone();
    let read = card_refs(&state, tenant_id, std::slice::from_ref(&card))
        .await
        .map(|refs| to_api(card, &refs, Some((no_entries, today()))));
    Ok(committed_write(StatusCode::CREATED, read, || {
        to_api(known, &Refs::default(), Some((no_entries, today())))
    }))
}

/// Update a rate card
///
/// A JSON merge patch of its name and what unmatched usage does. What it rates never changes.
#[utoipa::path(
    patch,
    tag = "Rate cards",
    path = "/api/v2/rate-cards/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Rate card id or code")),
    request_body(content = UpdateRateCard, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated, with today's rates", body = RateCard),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_rate_card(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateRateCard>,
) -> Result<Response, Problem> {
    let id = ids::rate_card_id(&state, auth.tenant_id, &id_or_code).await?;
    if body
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(Problem::field("/name", "invalid", "A rate card has a name"));
    }
    if let Some(name) = body.name.as_deref() {
        crate::v2::bounds::name(name.trim(), "/name")?;
    }
    if body.name.is_none() && body.on_unmatched.is_none() {
        return Ok(Json(read_card(&state, auth.tenant_id, id, today()).await?).into_response());
    }
    state
        .store
        .update_rate_card(
            auth.as_actor(),
            auth.tenant_id,
            id,
            body.name,
            body.on_unmatched.map(unmatched),
        )
        .await?;
    Ok(written(&state, auth.tenant_id, id).await)
}

/// What a change of a rate card was: `created`, `rates_set` (a batch of rates and removals),
/// `updated` (renamed, or what unmatched usage does changed), `archived` or `unarchived`.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateCardChangeKind {
    Created,
    /// A batch of rates and removals (`PATCH …/rates`).
    RatesSet,
    /// Renamed, or what unmatched usage does changed.
    Updated,
    Archived,
    Unarchived,
}

/// Who made a change: `system` (Meteroid itself), a dashboard `user`, an `api_token`, a
/// `customer` in its portal, a `quote_recipient`, or an `oauth_app` acting for a connected
/// account.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    System,
    User,
    ApiToken,
    Customer,
    QuoteRecipient,
    OauthApp,
}

/// Who made a change.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ChangeActor {
    #[serde(rename = "type")]
    pub kind: ActorKind,
    /// The user's, API key's or customer's id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// A key's rate in a currency as a batch changed it, from the batch's day.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct RateChange {
    /// A pattern per dimension, in the card's order.
    pub key: Vec<String>,
    /// None on a credit unit card.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// The rate the day before; none when the key had none.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub from: Option<Decimal>,
    /// None: the key is removed from the day.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub to: Option<Decimal>,
}

/// One change of a rate card, from its activity.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct RateCardChange {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: common_domain::ids::EntityActivityId,
    pub kind: RateCardChangeKind,
    pub occurred_at: ApiDateTime,
    pub actor: ChangeActor,
    /// The day a batch takes effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_from: Option<NaiveDate>,
    /// What a batch changed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rates: Vec<RateChange>,
    /// Scheduled rates a batch withdrew before their day.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub withdrawn: Vec<Rate>,
}

crate::v2_list!(RateCardChangeList, RateCardChange);

fn change_cursor(cursor: ActivityCursor) -> String {
    let micros = cursor.occurred_at.and_utc().timestamp_micros();
    let mut bytes = [0u8; 24];
    bytes[..8].copy_from_slice(&micros.to_be_bytes());
    bytes[8..].copy_from_slice(cursor.id.as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

fn read_change_cursor(raw: &str) -> Result<ActivityCursor, Problem> {
    let refused = || Problem::field("after", "invalid", "Not a cursor of this list");
    let bytes = URL_SAFE_NO_PAD.decode(raw.trim()).map_err(|_| refused())?;
    if bytes.len() != 24 {
        return Err(refused());
    }
    let mut micros = [0u8; 8];
    micros.copy_from_slice(&bytes[..8]);
    let occurred_at = chrono::DateTime::from_timestamp_micros(i64::from_be_bytes(micros))
        .ok_or_else(refused)?
        .naive_utc();
    let id = uuid::Uuid::from_slice(&bytes[8..]).map_err(|_| refused())?;
    Ok(ActivityCursor { occurred_at, id })
}

/// List a rate card's changes
///
/// Newest first: its creation, each batch of rates with the rate each key had before, and each
/// rename, with who made it. Paged by cursor like every list.
#[utoipa::path(
    get,
    tag = "Rate cards",
    path = "/api/v2/rate-cards/{id_or_code}/changes",
    params(
        ("id_or_code" = String, Path, description = "Rate card id or code"),
        PageQuery,
    ),
    responses(
        (status = 200, description = "A page of the card's changes, newest first", body = RateCardChangeList),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_rate_card_changes(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<RateCardChangeList>, Problem> {
    let id = ids::rate_card_id(&state, auth.tenant_id, &id_or_code).await?;
    let limit = query.limit()?;
    let before = query
        .after
        .as_deref()
        .filter(|after| !after.trim().is_empty())
        .map(read_change_cursor)
        .transpose()?;
    let page = state
        .store
        .list_entity_activities(
            auth.tenant_id,
            &EntityType::RateCard.to_string(),
            id.as_uuid(),
            before,
            u32::try_from(limit).unwrap_or(u32::MAX),
        )
        .await?;
    let data = page
        .items
        .into_iter()
        .filter_map(|entry| {
            let kind = match entry.activity_type.parse::<ActivityType>().ok()? {
                ActivityType::RateCardCreated => RateCardChangeKind::Created,
                ActivityType::RateCardRatesChanged => RateCardChangeKind::RatesSet,
                // An archive is recorded as an update saying so.
                ActivityType::EntityUpdated => match entry
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("archived"))
                    .and_then(serde_json::Value::as_bool)
                {
                    Some(true) => RateCardChangeKind::Archived,
                    Some(false) => RateCardChangeKind::Unarchived,
                    None => RateCardChangeKind::Updated,
                },
                _ => return None,
            };
            let changed: domain::RateCardRatesChanged = match kind {
                RateCardChangeKind::RatesSet => entry
                    .metadata
                    .clone()
                    .and_then(|metadata| serde_json::from_value(metadata).ok())
                    .unwrap_or_default(),
                _ => domain::RateCardRatesChanged::default(),
            };
            Some(RateCardChange {
                id: entry.id,
                kind,
                occurred_at: entry.occurred_at.into(),
                actor: ChangeActor {
                    kind: match entry.actor_type {
                        ActorType::System => ActorKind::System,
                        ActorType::User => ActorKind::User,
                        ActorType::ApiToken => ActorKind::ApiToken,
                        ActorType::Customer => ActorKind::Customer,
                        ActorType::QuoteRecipient => ActorKind::QuoteRecipient,
                        ActorType::OAuthApp => ActorKind::OauthApp,
                    },
                    id: entry.actor_uuid.map(|id| id.to_string()),
                },
                effective_from: changed.effective_from,
                rates: changed
                    .changes
                    .into_iter()
                    .map(|change| RateChange {
                        key: change.key,
                        currency: change.currency,
                        from: change.from.map(|rate| rate.normalize()),
                        to: change.to.map(|rate| rate.normalize()),
                    })
                    .collect(),
                withdrawn: changed
                    .withdrawn
                    .into_iter()
                    .map(|entry| Rate {
                        key: entry.key,
                        currency: entry.currency,
                        rate: entry.rate.map(|rate| rate.normalize()),
                        effective_from: entry.effective_from,
                        label: entry.label,
                    })
                    .collect(),
            })
        })
        .collect();
    let next_cursor = page.next_cursor.map(change_cursor);
    Ok(Json(RateCardChangeList {
        data,
        has_more: next_cursor.is_some(),
        next_cursor,
    }))
}

/// Archive a rate card
///
/// Off the catalog for new prices (a plan draft, an add-on or a subscription's own charge
/// pricing by it is refused); the prices it rates keep rating, and its rates can still be set.
/// Recorded in its changes.
#[utoipa::path(
    post,
    tag = "Rate cards",
    path = "/api/v2/rate-cards/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Rate card id or code")),
    responses(
        (status = 200, description = "Archived, with today's rates", body = RateCard),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_rate_card(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    set_archived(&state, &auth, &id_or_code, true).await
}

/// Unarchive a rate card
///
/// Back on the catalog for new prices. Recorded in its changes; unarchiving a live card changes
/// nothing.
#[utoipa::path(
    post,
    tag = "Rate cards",
    path = "/api/v2/rate-cards/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Rate card id or code")),
    responses(
        (status = 200, description = "Unarchived, with today's rates", body = RateCard),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_rate_card(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    set_archived(&state, &auth, &id_or_code, false).await
}

async fn set_archived(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id_or_code: &str,
    archived: bool,
) -> Result<Response, Problem> {
    let id = ids::rate_card_id(state, auth.tenant_id, id_or_code).await?;
    state
        .store
        .set_rate_card_archived(auth.as_actor(), auth.tenant_id, id, archived)
        .await?;
    Ok(written(state, auth.tenant_id, id).await)
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_rate_cards, create_rate_card))
        .routes(routes!(get_rate_card, update_rate_card))
        .routes(routes!(set_rates))
        .routes(routes!(list_rate_card_changes))
        .routes(routes!(archive_rate_card))
        .routes(routes!(unarchive_rate_card))
}
