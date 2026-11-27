//! Pricing pages: each plan's copy for one, and a product line's ladder priced as a visitor sees
//! it, in one currency in one option. Sales-led plans show here too, never self-serve. Read by a public page later: nothing internal beyond plan
//! ids leaves here.
use std::collections::{HashMap, HashSet};

use crate::StoreResult;
use crate::domain::Price;
use crate::domain::entitlements::{ConfigValue, ResolvedEntitlement, ResolvedEntitlementValue};
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::plan_options::PlanOption;
use crate::domain::price_cells::{OptionPricing, sold_options};
use crate::domain::price_components::{OptionPrice, PriceComponent};
use crate::domain::price_headline::{Catalog, card_period, in_order, narrowed, priced};
use crate::domain::prices::{FeeStructure, extract_legacy_pricing};
use crate::domain::pricing_pages::{
    CardExtra, CardLevel, CardLevels, CardPrice, Highlight, LadderSeller, MAX_COPY_CHARS,
    MAX_HIDDEN_CHARGES, MAX_HIGHLIGHTS, MAX_ORDERED_CHARGES, PageOption, PlanPricingCharges,
    PlanPricingPage, PricingCard, PricingCharge, PricingPage, PricingPageRequest,
    RenderedHighlight, format_quantity, levels_more, money, monthly_equivalent, saving_percent,
};
use crate::errors::StoreError;
use crate::repositories::entitlements::{EntitlementsInterface, ResolveTarget};
use crate::repositories::purchase_links::validate_redirect_url;
use crate::store::{PgConn, Store};
use common_domain::ids::{
    BillableMetricId, FeatureId, PlanId, PlanVersionId, PriceComponentId, PriceId, ProductFamilyId,
    ProductId, TenantId,
};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::enums::{PlanStatusEnum, PlanTypeEnum};
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::plans::PlanRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::products::ProductRow;
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait PricingPagesInterface {
    async fn get_plan_pricing_page(
        &self,
        tenant_id: TenantId,
        plan_id: PlanId,
    ) -> StoreResult<PlanPricingPage>;

    /// The copy of each of the tenant's plans among `plan_ids`, in one read; a plan not the
    /// tenant's is left out.
    async fn get_plan_pricing_pages(
        &self,
        tenant_id: TenantId,
        plan_ids: &[PlanId],
    ) -> StoreResult<HashMap<PlanId, PlanPricingPage>>;

    /// The charges of the plan's published version (else its draft) its card can be led by or
    /// leave off, one per product: fixed fees first, then seats, usage, one-time fees and packs.
    async fn plan_pricing_charges(
        &self,
        tenant_id: TenantId,
        plan_id: PlanId,
    ) -> StoreResult<PlanPricingCharges>;

    /// Replaces the plan's copy. Marking it popular unmarks the other plans of its product line.
    /// A feature highlight is one the plan's published version grants, each at most once. The
    /// card's order and what it leaves off name products of the catalog.
    async fn update_plan_pricing_page(
        &self,
        tenant_id: TenantId,
        page: PlanPricingPage,
    ) -> StoreResult<PlanPricingPage>;

    /// A plan's live fields (`patch`), what it is sold for (`sold_for`, `Some(None)` the
    /// customer) and its copy (`page`) written together, in one transaction: all or none. A
    /// `patch` changing nothing is not written.
    async fn update_plan_listing(
        &self,
        actor: crate::domain::entity_activity::Actor,
        patch: crate::domain::PlanPatch,
        sold_for: Option<Option<common_domain::ids::ResourceTypeId>>,
        page: Option<PlanPricingPage>,
        trial: Option<crate::domain::trials::PlanTrialChange>,
    ) -> StoreResult<()>;

    /// The product line's active ladder priced in one currency in one option (see
    /// [`PricingPageRequest`]).
    async fn pricing_page(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        request: PricingPageRequest,
    ) -> StoreResult<PricingPage>;
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

fn highlights_of(row: &PlanRow) -> StoreResult<Vec<Highlight>> {
    serde_json::from_value(row.pricing_highlights.clone()).map_err(|e| {
        Report::new(StoreError::SerdeError(
            "Failed to deserialize pricing highlights".to_owned(),
            e,
        ))
    })
}

fn page_of(row: &PlanRow) -> StoreResult<PlanPricingPage> {
    Ok(PlanPricingPage {
        plan_id: row.id,
        tagline: row.pricing_tagline.clone(),
        popular: row.pricing_popular,
        cta_label: row.pricing_cta_label.clone(),
        highlights: highlights_of(row)?,
        sales_led: row.pricing_sales_led,
        hide_price: row.pricing_hide_price,
        contact_url: row.pricing_contact_url.clone(),
        product_order: order_of(row),
        hidden_product_ids: hidden_of(row),
    })
}

fn order_of(row: &PlanRow) -> Vec<ProductId> {
    row.pricing_product_order
        .iter()
        .flatten()
        .copied()
        .collect()
}

fn hidden_of(row: &PlanRow) -> Vec<ProductId> {
    row.pricing_hidden_product_ids
        .iter()
        .flatten()
        .copied()
        .collect()
}

/// Trimmed, none when blank.
fn trimmed(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Trimmed, none when blank, refused past the limit.
fn checked_copy(value: Option<String>, what: &str) -> StoreResult<Option<String>> {
    let value = trimmed(value);
    if value
        .as_ref()
        .is_some_and(|v| v.chars().count() > MAX_COPY_CHARS)
    {
        return Err(invalid(format!(
            "Keep the {what} under {MAX_COPY_CHARS} characters"
        )));
    }
    Ok(value)
}

fn checked_highlights(highlights: Vec<Highlight>) -> StoreResult<Vec<Highlight>> {
    if highlights.len() > MAX_HIGHLIGHTS {
        return Err(invalid(format!("Pick at most {MAX_HIGHLIGHTS} highlights")));
    }
    let mut features = HashSet::new();
    highlights
        .into_iter()
        .map(|highlight| match highlight {
            Highlight::Feature(id) => {
                if !features.insert(id) {
                    return Err(invalid("Pick each feature once"));
                }
                Ok(Highlight::Feature(id))
            }
            Highlight::Text(text) => {
                let text = text.trim().to_owned();
                if text.is_empty() {
                    return Err(invalid("Enter the highlight's text"));
                }
                if text.chars().count() > MAX_COPY_CHARS {
                    return Err(invalid(format!(
                        "Keep each highlight under {MAX_COPY_CHARS} characters"
                    )));
                }
                Ok(Highlight::Text(text))
            }
        })
        .collect()
}

/// Each product once, in the order given.
fn distinct(ids: Vec<ProductId>) -> Vec<ProductId> {
    let mut checked: Vec<ProductId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !checked.contains(&id) {
            checked.push(id);
        }
    }
    checked
}

/// Each product ordered once; refused past the limit.
fn checked_order(order: Vec<ProductId>) -> StoreResult<Vec<ProductId>> {
    let checked = distinct(order);
    if checked.len() > MAX_ORDERED_CHARGES {
        return Err(invalid(format!(
            "Order at most {MAX_ORDERED_CHARGES} charges on the card"
        )));
    }
    Ok(checked)
}

/// Each product hidden once; refused past the limit.
fn checked_hidden(hidden: Vec<ProductId>) -> StoreResult<Vec<ProductId>> {
    let checked = distinct(hidden);
    if checked.len() > MAX_HIDDEN_CHARGES {
        return Err(invalid(format!(
            "Leave at most {MAX_HIDDEN_CHARGES} charges off the card"
        )));
    }
    Ok(checked)
}

/// `row` as it reads with `draft`'s copy, unsaved: what an editor previews. Blank copy is none
/// and what only a sales-led plan has is dropped, as saving reads them; nothing is refused.
fn with_draft(row: &mut PlanRow, draft: &PlanPricingPage) -> StoreResult<()> {
    let highlights: Vec<Highlight> = draft
        .highlights
        .iter()
        .filter_map(|highlight| match highlight {
            Highlight::Text(text) => trimmed(Some(text.clone())).map(Highlight::Text),
            feature => Some(feature.clone()),
        })
        .collect();
    row.pricing_highlights = serde_json::to_value(&highlights).map_err(|e| {
        Report::new(StoreError::SerdeError(
            "Failed to serialize pricing highlights".to_owned(),
            e,
        ))
    })?;
    row.pricing_tagline = trimmed(draft.tagline.clone());
    row.pricing_popular = draft.popular;
    row.pricing_cta_label = trimmed(draft.cta_label.clone());
    row.pricing_sales_led = draft.sales_led;
    row.pricing_hide_price = draft.hide_price;
    row.pricing_contact_url = trimmed(draft.contact_url.clone());
    row.pricing_product_order = draft.product_order.iter().copied().map(Some).collect();
    row.pricing_hidden_product_ids = draft.hidden_product_ids.iter().copied().map(Some).collect();
    Ok(())
}

/// A ladder plan with its published version, that version's options and components.
struct LadderPlan {
    plan: PlanRow,
    version: PlanVersionRow,
    options: Vec<PlanOption>,
    components: Vec<PriceComponent>,
}

impl LadderPlan {
    /// Whether nothing of it is priced: free in every currency.
    fn unpriced(&self) -> bool {
        self.components
            .iter()
            .all(|c| c.prices.is_empty() && c.legacy_pricing.is_none())
    }

    /// The options it sells in `currency` and shows: a hidden one is sold when named, never
    /// advertised.
    fn sold_in(&self, currency: &str) -> Vec<&PlanOption> {
        let mut sold = sold_options(&self.options, &self.components, currency);
        sold.retain(|option| !option.hidden);
        sold
    }
}

/// The option a card shows for `alias`, of those its plan `sold`: the one so named, else its
/// default (the first), then marked a fallback.
fn shown_option<'a>(
    sold: &[&'a PlanOption],
    alias: Option<&str>,
) -> Option<(&'a PlanOption, bool)> {
    match alias.and_then(|alias| sold.iter().find(|o| o.alias.eq_ignore_ascii_case(alias))) {
        Some(named) => Some((*named, false)),
        None => sold.first().map(|default| (*default, alias.is_some())),
    }
}

/// Under a card priced in its default for want of the option shown: how it bills when it sells
/// that one only ("Billed monthly"), else that option's name ("Annual only").
fn option_note(
    sold: &[&PlanOption],
    option: &PlanOption,
    period: Option<BillingPeriodEnum>,
) -> Option<String> {
    match sold {
        [_] => period.map(|period| {
            // As the page words a billed amount: "$1,490 billed yearly".
            let adverb = match period {
                BillingPeriodEnum::Monthly => "monthly",
                BillingPeriodEnum::Quarterly => "quarterly",
                BillingPeriodEnum::Semiannual => "every 6 months",
                BillingPeriodEnum::Annual => "yearly",
            };
            format!("Billed {adverb}")
        }),
        _ => Some(format!("{} only", option.name)),
    }
}

/// The options a visitor switches between, of those each priced plan `sold`: the ones two plans
/// sell at least, so a switch moves more than one card. None shared: those of the plans selling
/// several, else every one. In ladder then option order.
fn page_options(priced: &[&Vec<&PlanOption>]) -> Vec<PageOption> {
    fn distinct<'a>(options: impl Iterator<Item = &'a PlanOption>) -> Vec<PageOption> {
        let mut distinct: Vec<PageOption> = Vec::new();
        for option in options {
            if !distinct
                .iter()
                .any(|o| o.alias.eq_ignore_ascii_case(&option.alias))
            {
                distinct.push(PageOption {
                    alias: option.alias.clone(),
                    name: option.name.clone(),
                });
            }
        }
        distinct
    }
    let sellers = |alias: &str| {
        priced
            .iter()
            .filter(|sold| sold.iter().any(|o| o.alias.eq_ignore_ascii_case(alias)))
            .count()
    };
    let all = distinct(priced.iter().flat_map(|sold| sold.iter().copied()));
    let shared: Vec<PageOption> = all
        .iter()
        .filter(|o| sellers(&o.alias) >= 2)
        .cloned()
        .collect();
    if !shared.is_empty() {
        return shared;
    }
    let several = distinct(
        priced
            .iter()
            .filter(|sold| sold.len() > 1)
            .flat_map(|sold| sold.iter().copied()),
    );
    if several.is_empty() { all } else { several }
}

/// The card's price in `option`; none when the plan is not sold in it in the currency. Its
/// saving reads against `monthly`, another option of the plan billed monthly.
fn card_price(
    plan: &LadderPlan,
    option: Option<&PlanOption>,
    monthly: Option<&PlanOption>,
    currency: &str,
    catalog: &Catalog,
    precision: Option<u8>,
) -> Option<CardPrice> {
    if plan.unpriced() {
        return Some(CardPrice {
            amount: money(Decimal::ZERO, precision),
            period: None,
            monthly_equivalent: None,
            saving_percent: None,
            from_lowest: false,
            partial: false,
            extras: vec![],
            levels: None,
        });
    }
    let pricing = |option: &PlanOption| OptionPricing::new(option.id, currency);
    let option = pricing(option?);
    if !option.sells(&plan.components) {
        return None;
    }
    let components = narrowed(&plan.components, &option);
    let period = card_period(&components);
    let on_period = in_order(
        priced(&components, period, catalog),
        &order_of(&plan.plan),
        &hidden_of(&plan.plan),
    );
    let amount = money(on_period.fixed, precision);
    let shown = |amount: Decimal| amount.round_dp(u32::from(precision.unwrap_or(2)));
    let monthly = period
        .filter(|p| p.as_months() > 1)
        .and(monthly)
        .map(|monthly| narrowed(&plan.components, &pricing(monthly)))
        .filter(|monthly| card_period(monthly) == Some(BillingPeriodEnum::Monthly))
        .map(|monthly| priced(&monthly, Some(BillingPeriodEnum::Monthly), catalog))
        .filter(|monthly| !monthly.partial && monthly.from_lowest == on_period.from_lowest)
        .map(|monthly| shown(monthly.fixed));
    Some(CardPrice {
        monthly_equivalent: period
            .and_then(|period| monthly_equivalent(shown(on_period.fixed), period, precision)),
        saving_percent: match (period, monthly) {
            (Some(period), Some(monthly)) if !on_period.partial => {
                saving_percent(shown(on_period.fixed), period, monthly)
            }
            _ => None,
        },
        amount,
        period,
        from_lowest: on_period.from_lowest,
        partial: on_period.partial,
        extras: on_period
            .extras
            .into_iter()
            .map(|extra| CardExtra {
                amount: extra.amount.map(|a| money(a, precision)),
                ..extra
            })
            .collect(),
        // Each level read as the card's amount is: per month beside a longer period.
        levels: on_period.levels.map(|levels| CardLevels {
            levels: levels
                .levels
                .into_iter()
                .map(|level| CardLevel {
                    monthly_equivalent: period.and_then(|period| {
                        monthly_equivalent(shown(level.amount), period, precision)
                    }),
                    amount: money(level.amount, precision),
                    overage_rate: level.overage_rate.map(|rate| money(rate, precision)),
                    included: level.included,
                })
                .collect(),
            ..levels
        }),
    })
}

fn render_highlight(
    highlight: &Highlight,
    resolved: &HashMap<FeatureId, ResolvedEntitlement>,
) -> Option<RenderedHighlight> {
    let feature_id = match highlight {
        Highlight::Text(text) => {
            return Some(RenderedHighlight {
                text: text.clone(),
                feature_id: None,
            });
        }
        Highlight::Feature(id) => *id,
    };
    let entitlement = resolved.get(&feature_id)?;
    let name = &entitlement.feature.name;
    let text = match &entitlement.value {
        ResolvedEntitlementValue::Boolean { enabled } => enabled.then(|| name.clone())?,
        ResolvedEntitlementValue::Metered { enabled, limit, .. } => {
            if !enabled {
                return None;
            }
            match limit {
                Some(limit) => format!("{} {name}", format_quantity(*limit)),
                None => format!("Unlimited {name}"),
            }
        }
        ResolvedEntitlementValue::Config { value } => match value {
            ConfigValue::Number(number) => format!("{} {name}", format_quantity(*number)),
            ConfigValue::Boolean(enabled) => enabled.then(|| name.clone())?,
            ConfigValue::Text(text) => format!("{name}: {text}"),
            ConfigValue::Json(_) => name.clone(),
        },
    };
    Some(RenderedHighlight {
        text,
        feature_id: Some(feature_id),
    })
}

impl Store {
    /// The active, non-custom plans of the ladder with their published versions and components,
    /// read in a few bulk queries.
    async fn ladder_plans(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
    ) -> StoreResult<Vec<LadderPlan>> {
        let plans: Vec<PlanRow> = PlanRow::list_ladder(conn, tenant_id, product_family_id)
            .await?
            .into_iter()
            .filter(|plan| {
                matches!(plan.status, PlanStatusEnum::Active)
                    && plan.plan_type != PlanTypeEnum::Custom
            })
            .collect();
        let version_ids: Vec<PlanVersionId> =
            plans.iter().filter_map(|p| p.active_version_id).collect();
        if version_ids.is_empty() {
            return Ok(vec![]);
        }
        let mut versions: HashMap<PlanVersionId, PlanVersionRow> =
            PlanVersionRow::list_by_ids_and_tenant_id(conn, &version_ids, tenant_id)
                .await?
                .into_iter()
                .map(|version| (version.id, version))
                .collect();
        let mut rows =
            PriceComponentRow::get_by_plan_version_ids(conn, &version_ids, tenant_id).await?;
        let component_ids: Vec<PriceComponentId> =
            rows.values().flatten().map(|row| row.id).collect();
        let links = if component_ids.is_empty() {
            vec![]
        } else {
            PlanComponentPriceRow::list_by_component_ids(conn, &component_ids).await?
        };
        let price_ids: Vec<PriceId> = links.iter().map(|link| link.price_id).collect();
        let prices: HashMap<PriceId, Price> = if price_ids.is_empty() {
            HashMap::new()
        } else {
            PriceRow::list_by_ids(conn, &price_ids, tenant_id)
                .await?
                .into_iter()
                .map(|row| Price::try_from(row).map(|price| (price.id, price)))
                .collect::<Result<HashMap<_, _>, _>>()?
        };
        let mut prices_of: HashMap<PriceComponentId, Vec<Price>> = HashMap::new();
        let mut links_of: HashMap<PriceComponentId, Vec<OptionPrice>> = HashMap::new();
        for link in &links {
            if let Some(price) = prices.get(&link.price_id) {
                links_of
                    .entry(link.plan_component_id)
                    .or_default()
                    .push(OptionPrice {
                        option_id: link.option_id,
                        price_id: link.price_id,
                    });
                // A price the same in several options is one price.
                let prices = prices_of.entry(link.plan_component_id).or_default();
                if !prices.iter().any(|p| p.id == price.id) {
                    prices.push(price.clone());
                }
            }
        }
        let mut options_of = crate::services::subscriptions::currency::plan_options_of(
            conn,
            tenant_id,
            &version_ids,
        )
        .await?;

        let mut ladder = Vec::with_capacity(plans.len());
        for plan in plans {
            let Some(version) = plan.active_version_id.and_then(|id| versions.remove(&id)) else {
                continue;
            };
            let mut components = Vec::new();
            for row in rows.remove(&version.id).unwrap_or_default() {
                let legacy_fee = row.legacy_fee.clone();
                let mut component: PriceComponent = row.try_into()?;
                component.prices = prices_of.remove(&component.id).unwrap_or_default();
                component.option_prices = links_of.remove(&component.id).unwrap_or_default();
                if component.prices.is_empty()
                    && let Some(legacy_fee) = legacy_fee
                {
                    component.legacy_pricing = Some(extract_legacy_pricing(
                        &legacy_fee,
                        version.currency.clone(),
                    )?);
                }
                components.push(component);
            }
            ladder.push(LadderPlan {
                plan,
                options: options_of.remove(&version.id).unwrap_or_default(),
                version,
                components,
            });
        }
        Ok(ladder)
    }

    async fn catalog_of(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        plans: &[LadderPlan],
    ) -> StoreResult<Catalog> {
        // The metrics the charges bill, each charge reading its own structure.
        let metric_ids: Vec<BillableMetricId> = plans
            .iter()
            .flat_map(|p| p.components.iter())
            .filter_map(|c| c.structure().and_then(FeeStructure::metric_id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let metrics = if metric_ids.is_empty() {
            HashMap::new()
        } else {
            BillableMetricRow::get_by_ids(conn, &metric_ids, &tenant_id)
                .await?
                .into_iter()
                .map(|metric| (metric.id, metric))
                .collect()
        };
        Ok(Catalog { metrics })
    }
}

/// The currency shown: the one asked when the ladder is sold in it, else the tenant's when it
/// is, else the one selling most of it, else the tenant's.
fn chosen_currency(
    sold: &[(String, usize)],
    request: &PricingPageRequest,
    tenant_currency: &str,
) -> String {
    let sold_in = |currency: &str| {
        sold.iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(currency.trim()))
            .map(|(c, _)| c.clone())
    };
    request
        .currency
        .as_deref()
        .and_then(sold_in)
        .or_else(|| sold_in(tenant_currency))
        .or_else(|| {
            sold.iter()
                .rev()
                .max_by_key(|(_, count)| *count)
                .map(|(c, _)| c.clone())
        })
        .unwrap_or_else(|| tenant_currency.to_ascii_uppercase())
}

#[async_trait::async_trait]
impl PricingPagesInterface for Store {
    async fn get_plan_pricing_page(
        &self,
        tenant_id: TenantId,
        plan_id: PlanId,
    ) -> StoreResult<PlanPricingPage> {
        let mut conn = self.get_conn().await?;
        let row = PlanRow::find_by_id(&mut conn, plan_id, tenant_id).await?;
        page_of(&row)
    }

    async fn get_plan_pricing_pages(
        &self,
        tenant_id: TenantId,
        plan_ids: &[PlanId],
    ) -> StoreResult<HashMap<PlanId, PlanPricingPage>> {
        if plan_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut conn = self.get_conn().await?;
        PlanRow::list_by_ids(&mut conn, plan_ids, tenant_id)
            .await?
            .iter()
            .map(|row| -> StoreResult<(PlanId, PlanPricingPage)> { Ok((row.id, page_of(row)?)) })
            .collect()
    }

    async fn plan_pricing_charges(
        &self,
        tenant_id: TenantId,
        plan_id: PlanId,
    ) -> StoreResult<PlanPricingCharges> {
        let mut conn = self.get_conn().await?;
        let plan = PlanRow::find_by_id(&mut conn, plan_id, tenant_id).await?;
        let product_family_id = plan.product_family_id;
        let Some(version) = plan.active_version_id.or(plan.draft_version_id) else {
            return Ok(PlanPricingCharges {
                product_family_id,
                charges: vec![],
            });
        };
        let rows = PriceComponentRow::get_by_plan_version_ids(&mut conn, &[version], tenant_id)
            .await?
            .remove(&version)
            .unwrap_or_default();
        // Named as the version's charge, read as it bills; a product charged twice is chosen
        // once. An included product (no structure) charges nothing to show.
        let mut seen = HashSet::new();
        let mut charges: Vec<PricingCharge> = Vec::new();
        for row in rows {
            let Some(structure) =
                crate::domain::price_components::structure_of_json(row.fee_structure)?
            else {
                continue;
            };
            let Some(product_id) = row.product_id.filter(|id| seen.insert(*id)) else {
                continue;
            };
            charges.push(PricingCharge::of(product_id, row.name, &structure));
        }
        charges.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
        Ok(PlanPricingCharges {
            product_family_id,
            charges,
        })
    }

    async fn update_plan_listing(
        &self,
        actor: crate::domain::entity_activity::Actor,
        patch: crate::domain::PlanPatch,
        sold_for: Option<Option<common_domain::ids::ResourceTypeId>>,
        page: Option<PlanPricingPage>,
        trial: Option<crate::domain::trials::PlanTrialChange>,
    ) -> StoreResult<()> {
        use crate::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};
        use common_domain::ids::BaseId;

        let mut changed: Vec<&'static str> = Vec::new();
        if patch.name.is_some() {
            changed.push("name");
        }
        if patch.description.is_some() {
            changed.push("description");
        }
        if patch.self_service_rank.is_some() {
            changed.push("self_service_rank");
        }
        if patch.issues_license.is_some() {
            changed.push("issues_license");
        }
        let changes_row = !changed.is_empty() || patch.active_version_id.is_some();
        if sold_for.is_some() {
            changed.push("sold_for_resource_type_id");
        }
        let (plan_id, tenant_id) = (patch.id, patch.tenant_id);
        let row_patch: diesel_models::plans::PlanRowPatch = patch.into();
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                if changes_row {
                    row_patch.update(conn).await?;
                }
                if let Some(sold_for) = sold_for {
                    crate::repositories::plans::set_sold_for(conn, tenant_id, plan_id, sold_for)
                        .await?;
                }
                if !changed.is_empty() {
                    let activity = Activity::new(
                        ActivityType::EntityUpdated,
                        EntityType::Plan,
                        plan_id.as_uuid(),
                    )
                    .with_metadata(serde_json::json!({
                        "changes": changed
                            .iter()
                            .map(|field| serde_json::json!({ "field": field }))
                            .collect::<Vec<_>>(),
                    }));
                    self.internal
                        .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;
                }
                if let Some(page) = page {
                    if page.plan_id != plan_id {
                        return Err(invalid("The copy is another plan's"));
                    }
                    self.write_plan_pricing_page_tx(conn, tenant_id, page)
                        .await?;
                }
                match trial {
                    None => {}
                    Some(crate::domain::trials::PlanTrialChange::Terms(terms)) => {
                        crate::services::trials::catalog::set_plan_trial_tx(
                            &self.internal,
                            conn,
                            actor,
                            tenant_id,
                            plan_id,
                            terms,
                        )
                        .await?;
                    }
                    Some(crate::domain::trials::PlanTrialChange::Offer(offer)) => {
                        crate::services::trials::catalog::set_plan_trial_offer_tx(
                            &self.internal,
                            conn,
                            actor,
                            tenant_id,
                            plan_id,
                            Some(offer),
                        )
                        .await?;
                    }
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn update_plan_pricing_page(
        &self,
        tenant_id: TenantId,
        page: PlanPricingPage,
    ) -> StoreResult<PlanPricingPage> {
        self.transaction(|conn| {
            async move { self.write_plan_pricing_page_tx(conn, tenant_id, page).await }
                .scope_boxed()
        })
        .await
    }

    async fn pricing_page(
        &self,
        tenant_id: TenantId,
        product_family_id: ProductFamilyId,
        request: PricingPageRequest,
    ) -> StoreResult<PricingPage> {
        let mut conn = self.get_conn().await?;
        let family = ProductFamilyRow::find_by_id(&mut conn, product_family_id, tenant_id).await?;
        let tenant = TenantRow::find_by_id(&mut conn, tenant_id).await?;
        let mut plans = self
            .ladder_plans(&mut conn, tenant_id, product_family_id)
            .await?;
        if let Some(draft) = &request.draft {
            for plan in &mut plans {
                if plan.plan.id == draft.plan_id {
                    with_draft(&mut plan.plan, draft)?;
                } else if draft.popular {
                    // As saving it would: one popular plan per product line.
                    plan.plan.pricing_popular = false;
                }
            }
        }
        let catalog = self.catalog_of(&mut conn, tenant_id, &plans).await?;

        let mut currencies: Vec<String> = plans
            .iter()
            .flat_map(|p| p.components.iter())
            .flat_map(|c| c.prices.iter())
            .map(|price| price.currency.trim().to_ascii_uppercase())
            .collect();
        currencies.sort();
        currencies.dedup();
        let priced_plans: Vec<&LadderPlan> = plans.iter().filter(|p| !p.unpriced()).collect();
        let sold: Vec<(String, usize)> = currencies
            .into_iter()
            .map(|currency| {
                let count = priced_plans
                    .iter()
                    .filter(|p| !p.sold_in(&currency).is_empty())
                    .count();
                (currency, count)
            })
            .filter(|(_, count)| *count > 0)
            .collect();
        let currency = chosen_currency(&sold, &request, &tenant.reporting_currency);
        let precision = crate::constants::Currencies::resolve_currency_precision(&currency);

        let plan_options: Vec<Vec<&PlanOption>> =
            plans.iter().map(|plan| plan.sold_in(&currency)).collect();
        let priced: Vec<&Vec<&PlanOption>> = plans
            .iter()
            .zip(&plan_options)
            .filter(|(plan, _)| !plan.unpriced())
            .map(|(_, sold)| sold)
            .collect();
        let options = page_options(&priced);
        let shown = request
            .option
            .as_deref()
            .and_then(|asked| {
                options
                    .iter()
                    .find(|o| o.alias.eq_ignore_ascii_case(asked.trim()))
            })
            .or_else(|| options.first())
            .map(|o| o.alias.clone());

        // How the option shown bills where it is sold: a plan of one option billing alike needs
        // no note ("Billed monthly" under Monthly says nothing).
        let shown_period = shown.as_deref().and_then(|alias| {
            plans
                .iter()
                .zip(plan_options.iter())
                .find_map(|(plan, sold)| {
                    sold.iter()
                        .find(|o| o.alias.eq_ignore_ascii_case(alias))
                        .and_then(|o| {
                            card_period(&narrowed(
                                &plan.components,
                                &OptionPricing::new(o.id, &currency),
                            ))
                        })
                })
        });
        // Where a card's usage levels go past their largest ("More").
        let sellers: Vec<LadderSeller<'_>> = plans
            .iter()
            .map(|plan| LadderSeller {
                name: &plan.plan.name,
                sales_led: plan.plan.pricing_sales_led,
                contact_url: plan.plan.pricing_contact_url.as_deref(),
            })
            .collect();
        let plan_ids: Vec<PlanId> = plans.iter().map(|plan| plan.plan.id).collect();
        let trials =
            crate::services::trials::catalog::plan_trials_tx(&mut conn, tenant_id, &plan_ids)
                .await?;
        let mut cards: Vec<PricingCard> = Vec::with_capacity(plans.len());
        let mut not_sold = Vec::new();
        for (at, (plan, sold)) in plans.iter().zip(plan_options).enumerate() {
            // Not buyable in the currency: off the page, as a visitor could not take it.
            if sold.is_empty() && !plan.unpriced() && !plan.plan.pricing_sales_led {
                not_sold.push(plan.plan.name.clone());
                continue;
            }
            let highlights = highlights_of(&plan.plan)?;
            // Resolved per plan: entitlements have no bulk resolution by version yet.
            let resolved: HashMap<FeatureId, ResolvedEntitlement> = if highlights
                .iter()
                .any(|h| matches!(h, Highlight::Feature(_)))
            {
                self.resolve_entitlements_for_entity(
                    &mut conn,
                    tenant_id,
                    ResolveTarget::PlanVersion(plan.version.id),
                )
                .await?
                .into_iter()
                .map(|entitlement| (entitlement.feature.id, entitlement))
                .collect()
            } else {
                HashMap::new()
            };

            let (option, fallback) = match shown_option(&sold, shown.as_deref()) {
                Some((option, fallback)) => (Some(option), fallback),
                None => (None, false),
            };
            // The saving reads against the option billing the same plan monthly.
            let monthly = sold.iter().copied().find(|o| {
                let cells = narrowed(&plan.components, &OptionPricing::new(o.id, &currency));
                Some(o.id) != option.map(|shown| shown.id)
                    && card_period(&cells) == Some(BillingPeriodEnum::Monthly)
            });
            let hide_price = plan.plan.pricing_sales_led && plan.plan.pricing_hide_price;
            // A hidden price never leaves here: the page is public.
            let mut price = card_price(plan, option, monthly, &currency, &catalog, precision)
                .filter(|_| !hide_price);
            if let Some(levels) = price.as_mut().and_then(|price| price.levels.as_mut()) {
                levels.more = levels_more(&sellers, at);
            }
            let option_note = match (option, &price) {
                (Some(_), Some(price))
                    if fallback && sold.len() == 1 && price.period == shown_period =>
                {
                    None
                }
                (Some(option), Some(price)) if fallback && !plan.unpriced() => {
                    option_note(&sold, option, price.period)
                }
                _ => None,
            };
            let free = price
                .as_ref()
                .is_some_and(|p| p.amount.is_zero() && p.extras.is_empty() && !p.partial);
            let includes_plan_name = cards.last().map(|card| card.plan_name.clone());
            cards.push(PricingCard {
                plan_id: plan.plan.id,
                plan_name: plan.plan.name.clone(),
                tagline: plan.plan.pricing_tagline.clone(),
                popular: plan.plan.pricing_popular,
                cta_label: plan.plan.pricing_cta_label.clone(),
                free,
                price,
                options: sold.iter().map(|o| o.alias.clone()).collect(),
                includes_plan_name,
                highlights: highlights
                    .iter()
                    .filter_map(|h| render_highlight(h, &resolved))
                    .collect(),
                trial_days: trials
                    .get(&plan.plan.id)
                    .map(|trial| trial.offer.duration_days),
                sales_led: plan.plan.pricing_sales_led,
                hide_price,
                contact_url: plan
                    .plan
                    .pricing_contact_url
                    .clone()
                    .filter(|_| plan.plan.pricing_sales_led),
                option_note,
            });
        }

        Ok(PricingPage {
            product_family_id,
            product_family_name: family.name,
            currency,
            option: shown,
            options,
            currencies: sold.into_iter().map(|(currency, _)| currency).collect(),
            cards,
            not_sold,
        })
    }
}

impl Store {
    /// `update_plan_pricing_page` inside the caller's transaction.
    pub(crate) async fn write_plan_pricing_page_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        page: PlanPricingPage,
    ) -> StoreResult<PlanPricingPage> {
        let tagline = checked_copy(page.tagline, "tagline")?;
        let cta_label = checked_copy(page.cta_label, "button's text")?;
        let highlights = checked_highlights(page.highlights)?;
        let product_order = checked_order(page.product_order)?;
        let hidden_product_ids = checked_hidden(page.hidden_product_ids)?;
        let sales_led = page.sales_led;
        // Hiding the price and the contact link only read for a sales-led plan (the cards gate
        // them), but stay stored, so a plan sold self-serve for a while keeps them.
        let hide_price = page.hide_price;
        let contact_url = page
            .contact_url
            .map(|url| url.trim().to_owned())
            .filter(|url| !url.is_empty());
        validate_redirect_url("The contact link", contact_url.as_deref()).map_err(Report::new)?;
        let highlights_json = serde_json::to_value(&highlights).map_err(|e| {
            Report::new(StoreError::SerdeError(
                "Failed to serialize pricing highlights".to_owned(),
                e,
            ))
        })?;
        let (plan_id, popular) = (page.plan_id, page.popular);
        let plan = PlanRow::find_by_id(conn, plan_id, tenant_id).await?;
        let chosen: Vec<ProductId> = distinct(
            product_order
                .iter()
                .chain(hidden_product_ids.iter())
                .copied()
                .collect(),
        );
        if !chosen.is_empty() {
            // Products, not charges: the choice holds in every version pricing them.
            let known: HashSet<ProductId> = ProductRow::list_by_ids(conn, &chosen, tenant_id)
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            if chosen.iter().any(|id| !known.contains(id)) {
                return Err(invalid("Pick charges among the products of the catalog"));
            }
        }
        let feature_ids: Vec<FeatureId> = highlights
            .iter()
            .filter_map(|h| match h {
                Highlight::Feature(id) => Some(*id),
                Highlight::Text(_) => None,
            })
            .collect();
        if !feature_ids.is_empty() {
            // Only what the published version grants: a highlight shows its value.
            let granted: HashMap<FeatureId, ResolvedEntitlement> = match plan.active_version_id {
                Some(version) => self
                    .resolve_entitlements_for_entity(
                        conn,
                        tenant_id,
                        ResolveTarget::PlanVersion(version),
                    )
                    .await?
                    .into_iter()
                    .map(|entitlement| (entitlement.feature.id, entitlement))
                    .collect(),
                None => HashMap::new(),
            };
            if feature_ids
                .iter()
                .any(|id| render_highlight(&Highlight::Feature(*id), &granted).is_none())
            {
                return Err(invalid(
                    "Pick highlights among the features the plan's published version grants",
                ));
            }
        }
        if popular {
            // Locked, so two plans marked at once never both hold the flag.
            PlanRow::lock_by_product_family(conn, plan.product_family_id, tenant_id).await?;
            PlanRow::clear_pricing_popular(conn, tenant_id, plan.product_family_id, plan.id)
                .await?;
        }
        let row = PlanRow::update_pricing_page(
            conn,
            plan.id,
            tenant_id,
            tagline,
            popular,
            cta_label,
            highlights_json,
            sales_led,
            hide_price,
            contact_url,
            product_order,
            hidden_product_ids,
        )
        .await?;
        page_of(&row)
    }
}
