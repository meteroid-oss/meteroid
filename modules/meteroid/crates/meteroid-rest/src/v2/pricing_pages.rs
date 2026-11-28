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
use crate::v2::trials::AfterTrial;
use axum::Extension;
use axum::Json;
use axum::extract::State;
use chrono::NaiveDate;
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

/// An option of the page's switch ("Monthly", "Annual").
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PageOption {
    /// The option's alias, as `?option=` takes it.
    pub alias: String,
    /// What a visitor reads.
    pub name: String,
}

/// What an item of a card prices: `base`, the fixed fees billed on the card's period, summed;
/// `per_seat`, per seat of a managed quantity; `usage`, metered usage; `one_time`, a fee billed
/// once; `credit_pack`, a pack of credit; `recurring`, a fixed fee billed on its own period.
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
    /// What it prices.
    pub kind: ExtraKind,
    /// The charge's name; empty for the base price.
    pub name: String,
    /// A decimal string in the page's currency: the base price; per seat or unit; once; a
    /// pack's. None for usage at a rate card's or by-dimension rates, and levels whose overage
    /// varies.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub amount: Option<Decimal>,
    /// `amount` is the lowest rate of its tiers: "from".
    pub from_lowest: bool,
    /// Units free before `amount` applies ("3 seats included").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<u64>,
    /// `amount` is per this many units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_size: Option<u64>,
    /// What a unit is called ("seat", "request").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// How often a recurring item bills.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Cadence>,
    /// The card's price: the others read beside it.
    pub lead: bool,
}

/// A usage level a visitor may pick on a card.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CardLevel {
    /// The usage included each period, in the metric's unit.
    pub included: u64,
    /// The card's amount with this level, a decimal string in the page's currency.
    #[schema(value_type = String, format = "decimal")]
    pub amount: Decimal,
    /// `amount` per month, for a card billed on a longer period.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub monthly_equivalent: Option<Decimal>,
    /// Per unit used above `included`; none when priced in a credit unit or left off the card.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub overage_rate: Option<Decimal>,
}

/// Where the levels lead past their largest: sales.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct LevelsMore {
    /// Where to contact sales.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_url: Option<String>,
    /// The sales-led plan of the ladder it leads to; none when it is the card's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
}

/// The usage levels a visitor picks between.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CardLevels {
    /// The charge's name.
    pub name: String,
    /// By the usage they include, smallest first.
    pub levels: Vec<CardLevel>,
    /// What the charge counts, as its metric names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    /// The metric's unit, for one ("token").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_singular: Option<String>,
    /// The metric's unit, for several ("tokens").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_plural: Option<String>,
    /// A last stop past the largest level, sold by contacting sales; none when the ladder sells
    /// nothing so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more: Option<LevelsMore>,
}

/// What a card reads as its price.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CardPrice {
    /// A decimal string in the page's currency: the card's lead price.
    #[schema(value_type = String, format = "decimal")]
    pub amount: Decimal,
    /// None for a plan billing only usage or one-time charges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Cadence>,
    /// `amount` per month, for a price billed on a longer period.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub monthly_equivalent: Option<Decimal>,
    /// Whole percent saved paying `amount` each `period` rather than the monthly price each
    /// month, rounded down; none unless something is saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saving_percent: Option<u32>,
    /// The amount is the lowest of several levels: "from".
    pub from_lowest: bool,
    /// A charge could not be shown: never read as free.
    pub partial: bool,
    /// The card's items in the order they read, one of them its `lead`.
    pub extras: Vec<CardExtra>,
    /// The usage levels a visitor picks between, the card's amount reading the lowest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<CardLevels>,
}

/// A plan of the ladder as its card reads.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PricingCard {
    /// The plan, without version or option.
    pub plan: PlanRef,
    /// The plan's name.
    pub name: String,
    /// The plan's listing tagline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    /// Highlighted.
    pub popular: bool,
    /// The label of the card's button.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cta_label: Option<String>,
    /// Its price is 0 with no other item: nothing to pay.
    pub free: bool,
    /// None when the plan is not sold in the currency, or its price is hidden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<CardPrice>,
    /// The aliases of the options the plan sells in the currency.
    pub options: Vec<String>,
    /// "Everything in <plan>, plus".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub includes_plan_name: Option<String>,
    /// The lines listed on the card.
    pub highlights: Vec<String>,
    /// The trial's length in days, when the plan has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_days: Option<u32>,
    /// With a trial started today: what is billed each period once it ends, the card's base
    /// price; none when the price is not shown or has no base.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_trial: Option<AfterTrial>,
    /// Sold by contacting sales.
    pub sales_led: bool,
    /// The price is not shown: `price` is none and the card reads "Custom pricing".
    pub hide_price: bool,
    /// Where a sales-led card's button leads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_url: Option<String>,
    /// Under the price when the plan does not sell the option shown ("Billed monthly").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_note: Option<String>,
}

/// A product family's pricing page: its self-serve ladder, one card per plan, priced in one
/// currency and one option.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PricingPage {
    /// The product family.
    pub family: ProductFamilyRef,
    /// The product family's name.
    pub name: String,
    /// The currency the page is priced in (ISO 4217).
    pub currency: String,
    /// The alias of the option shown; none when no plan is sold in the currency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    /// The options a visitor switches between.
    pub options: Vec<PageOption>,
    /// The currencies a visitor switches between: those the ladder is sold in.
    pub currencies: Vec<String>,
    /// One per plan of the ladder sold in the currency or by contacting sales, in ladder order.
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

/// A card's fixed fees billed from the end of a trial started `today`, per cadence: its base
/// price, then the fees billed on their own cadence. Seats and usage are left out.
fn after_trial(card: &domain::PricingCard, currency: &str, today: NaiveDate) -> Option<AfterTrial> {
    use crate::v2::trials::RecurringAmount;
    use common_utils::decimals::ToSubunit;
    let days = card.trial_days.filter(|days| *days > 0)?;
    let price = card.price.as_ref()?;
    let own = price.period?;
    let precision = meteroid_store::constants::Currencies::resolve_currency_precision(currency)?;
    let mut amounts: Vec<(meteroid_store::domain::enums::BillingPeriodEnum, i64)> = Vec::new();
    for extra in &price.extras {
        let period = match extra.kind {
            domain::CardExtraKind::Base => own,
            domain::CardExtraKind::Recurring => match extra.period {
                Some(period) => period,
                None => continue,
            },
            _ => continue,
        };
        let amount = extra.amount?.to_subunit_opt(precision)?;
        match amounts.iter_mut().find(|(at, _)| *at == period) {
            Some((_, sum)) => *sum += amount,
            None => amounts.push((period, amount)),
        }
    }
    if !amounts.iter().any(|(period, _)| *period == own) {
        return None;
    }
    amounts.retain(|(_, amount)| *amount != 0);
    amounts.sort_by_key(|(period, _)| (*period != own, period.as_months()));
    Some(AfterTrial {
        currency: currency.to_owned(),
        starts_on: today.checked_add_days(chrono::Days::new(u64::from(days)))?,
        recurring: amounts
            .into_iter()
            .map(|(period, amount)| RecurringAmount {
                amount,
                interval: period.into(),
            })
            .collect(),
    })
}

fn card(
    card: domain::PricingCard,
    codes: &HashMap<PlanId, String>,
    currency: &str,
    today: NaiveDate,
) -> PricingCard {
    let after_trial = after_trial(&card, currency, today);
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
        after_trial,
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
    let today = common_clock::now().date_naive();
    let currency = page.currency.clone();
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
        cards: page
            .cards
            .into_iter()
            .map(|c| card(c, &codes, &currency, today))
            .collect(),
    })
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_pricing_page))
}
