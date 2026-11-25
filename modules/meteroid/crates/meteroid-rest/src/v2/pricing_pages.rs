//! `GET /api/v2/pricing-pages/{family_code}`: a product family's pricing page as the embeddable page
//! renders it — its self-serve ladder priced in one currency and one option. Read-only; a site's
//! backend reads it with its API key and caches it. A browser reads the same page with a
//! publishable key on the client API (`client::public`), rendered here.
use crate::AppState;
use crate::v2::charges::Cadence;
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::problem::Problem;
use crate::v2::refs::{PlanRef, ProductFamilyRef};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{PlanId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::domain::pricing_pages as domain;
use meteroid_store::repositories::{
    CatalogCodesInterface, PricingPagesInterface, ProductFamilyInterface,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct PricingPageQuery {
    /// A currency the ladder is sold in; the default when left out or not sold in.
    pub currency: Option<String>,
    /// An option alias of the page's switch; the first when left out or not on it.
    pub option: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PageOption {
    pub alias: String,
    pub name: String,
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtraKind {
    /// The fixed fees billed on the card's period, summed.
    Base,
    PerSeat,
    Usage,
    OneTime,
    CreditPack,
    /// A fixed fee billed on its own period, beside the card's amount.
    Recurring,
}

/// An item of a card, in the order it reads.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CardExtra {
    pub kind: ExtraKind,
    /// The charge's name; empty for the base price.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub amount: Option<Decimal>,
    /// `amount` is the lowest rate of its tiers.
    pub from_lowest: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<u64>,
    /// `amount` is per this many units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Cadence>,
    /// The card's price: the others read beside it.
    pub lead: bool,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CardLevel {
    pub included: u64,
    #[schema(value_type = String, format = "decimal")]
    pub amount: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub monthly_equivalent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub overage_rate: Option<Decimal>,
}

/// Where the levels lead past their largest: sales.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct LevelsMore {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
}

/// The usage levels a visitor picks between.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CardLevels {
    pub name: String,
    pub levels: Vec<CardLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_singular: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_plural: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more: Option<LevelsMore>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CardPrice {
    #[schema(value_type = String, format = "decimal")]
    pub amount: Decimal,
    /// None for a plan billing only usage or one-time charges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Cadence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub monthly_equivalent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saving_percent: Option<u32>,
    /// The amount is the lowest of several levels: "from".
    pub from_lowest: bool,
    /// A charge could not be shown: never read as free.
    pub partial: bool,
    pub extras: Vec<CardExtra>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<CardLevels>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PricingCard {
    pub plan: PlanRef,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    pub popular: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cta_label: Option<String>,
    pub free: bool,
    /// None when the plan is not sold in the currency, or its price is hidden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<CardPrice>,
    /// The aliases of the options the plan sells in the currency.
    pub options: Vec<String>,
    /// "Everything in <plan>, plus".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_plan_name: Option<String>,
    pub highlights: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_days: Option<u32>,
    pub sales_led: bool,
    pub hide_price: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_url: Option<String>,
    /// Under the price when the plan does not sell the option shown ("Billed monthly").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_note: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PricingPage {
    pub family: ProductFamilyRef,
    pub name: String,
    pub currency: String,
    /// The alias of the option shown; none when no plan is sold in the currency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    /// The options a visitor switches between.
    pub options: Vec<PageOption>,
    /// The currencies a visitor switches between.
    pub currencies: Vec<String>,
    pub cards: Vec<PricingCard>,
}

fn kind(kind: domain::CardExtraKind) -> ExtraKind {
    match kind {
        domain::CardExtraKind::Base => ExtraKind::Base,
        domain::CardExtraKind::PerSeat => ExtraKind::PerSeat,
        domain::CardExtraKind::Usage => ExtraKind::Usage,
        domain::CardExtraKind::OneTime => ExtraKind::OneTime,
        domain::CardExtraKind::CreditPack => ExtraKind::CreditPack,
        domain::CardExtraKind::Recurring => ExtraKind::Recurring,
    }
}

fn card(card: domain::PricingCard, codes: &HashMap<PlanId, String>) -> PricingCard {
    PricingCard {
        plan: PlanRef {
            id: card.plan_id,
            code: codes
                .get(&card.plan_id)
                .cloned()
                .unwrap_or_else(|| card.plan_id.to_string()),
            version: None,
            option: None,
        },
        name: card.plan_name,
        tagline: card.tagline,
        popular: card.popular,
        cta_label: card.cta_label,
        free: card.free,
        price: card.price.map(|price| CardPrice {
            amount: price.amount,
            period: price.period.map(Into::into),
            monthly_equivalent: price.monthly_equivalent,
            saving_percent: price.saving_percent,
            from_lowest: price.from_lowest,
            partial: price.partial,
            extras: price
                .extras
                .into_iter()
                .map(|extra| CardExtra {
                    kind: kind(extra.kind),
                    name: extra.name,
                    amount: extra.amount,
                    from_lowest: extra.from_lowest,
                    included: extra.included,
                    block_size: extra.block_size,
                    unit: extra.unit,
                    period: extra.period.map(Into::into),
                    lead: extra.lead,
                })
                .collect(),
            levels: price.levels.map(|levels| CardLevels {
                name: levels.name,
                levels: levels
                    .levels
                    .into_iter()
                    .map(|level| CardLevel {
                        included: level.included,
                        amount: level.amount,
                        monthly_equivalent: level.monthly_equivalent,
                        overage_rate: level.overage_rate,
                    })
                    .collect(),
                metric_name: levels.metric_name,
                unit_singular: levels.unit_singular,
                unit_plural: levels.unit_plural,
                more: levels.more.map(|more| LevelsMore {
                    contact_url: more.contact_url,
                    plan_name: more.plan_name,
                }),
            }),
        }),
        options: card.options,
        includes_plan_name: card.includes_plan_name,
        highlights: card
            .highlights
            .into_iter()
            .map(|highlight| highlight.text)
            .collect(),
        trial_days: card.trial_days,
        sales_led: card.sales_led,
        hide_price: card.hide_price,
        contact_url: card.contact_url,
        option_note: card.option_note,
    }
}

/// Get a pricing page
///
/// The product family's self-serve ladder priced in one currency and one option, card by card.
#[utoipa::path(
    get,
    tag = "Pricing pages",
    path = "/api/v2/pricing-pages/{family_code}",
    params(
        ("family_code" = String, Path, description = "Product family code (or id)"),
        PricingPageQuery,
    ),
    responses(
        (status = 200, description = "The pricing page", body = PricingPage),
        (status = 404, description = "No such product family", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_pricing_page(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(family): Path<String>,
    Query(query): Query<PricingPageQuery>,
) -> Result<Json<PricingPage>, Problem> {
    Ok(Json(render(&state, auth.tenant_id, &family, query).await?))
}

/// The tenant's pricing page of a product family (by code or id), as it is published: what a
/// visitor of the public page sees, a hidden price left out.
pub async fn render(
    state: &AppState,
    tenant_id: TenantId,
    family: &str,
    query: PricingPageQuery,
) -> Result<PricingPage, Problem> {
    let family_id = ids::product_family_id(state, tenant_id, family).await?;
    let family = state
        .store
        .find_product_family_by_id(family_id, tenant_id)
        .await?;
    let page = state
        .store
        .pricing_page(
            tenant_id,
            family_id,
            domain::PricingPageRequest {
                currency: query.currency.map(|c| c.trim().to_ascii_uppercase()),
                option: query.option,
                draft: None,
            },
        )
        .await?;
    let plan_ids: Vec<PlanId> = page.cards.iter().map(|card| card.plan_id).collect();
    let codes = state.store.plan_codes(tenant_id, &plan_ids).await?;
    Ok(PricingPage {
        family: ProductFamilyRef {
            id: family.id,
            code: family.code,
        },
        name: page.product_family_name,
        currency: page.currency,
        option: page.option,
        options: page
            .options
            .into_iter()
            .map(|option| PageOption {
                alias: option.alias,
                name: option.name,
            })
            .collect(),
        currencies: page.currencies,
        cards: page.cards.into_iter().map(|c| card(c, &codes)).collect(),
    })
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_pricing_page))
}
