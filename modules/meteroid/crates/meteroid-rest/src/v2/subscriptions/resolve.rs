//! What a subscription request names, resolved before anything is written: the plan version it
//! buys and the option and currency it is sold in, its charges by product code, seat counts,
//! levels, and the prices a charge is overridden at or added with.
use super::model::ChargeOverride;
use crate::AppState;
use crate::v2::charges::{
    Cadence, Charge, ChargePrice, EVERY_OPTION, OptionSet, PriceModel, StoredCharge, StoredPrice,
    render, to_domain,
};
use crate::v2::problem::{Problem, ProblemCode};
use crate::v2::references::{CatalogNames, resolve_refs};
use common_domain::ids::{BaseId, PlanId, ProductId, TenantId};
use http::StatusCode;
use meteroid_store::domain::managed_quantities::ManagedQuantity;
use meteroid_store::domain::plan_options::PlanOption;
use meteroid_store::domain::price_cells::{OptionPricing, currencies, sold_options};
use meteroid_store::domain::price_components::PriceInput;
use meteroid_store::domain::prices::FeeStructure;
use meteroid_store::domain::{FullPlan, PlanVersionFilter, PriceComponent};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::PlansInterface;
use rust_decimal::Decimal;
use std::str::FromStr;

/// A reference not found, read as the field naming it.
pub fn not_found_at(
    path: String,
    what: &'static str,
    raw: &str,
) -> impl FnOnce(Problem) -> Problem {
    let raw = raw.trim().to_owned();
    move |problem: Problem| {
        if problem.status == 404 {
            Problem::field(path, "not_found", format!("{what} {raw} not found"))
        } else {
            problem
        }
    }
}

/// The published version of the plan a request names: `version`, else the active one.
pub async fn published_version(
    state: &AppState,
    tenant_id: TenantId,
    plan_id: PlanId,
    version: Option<i32>,
    path: &str,
) -> Result<FullPlan, Problem> {
    let filter = match version {
        Some(number) => PlanVersionFilter::Version(number),
        None => PlanVersionFilter::Active,
    };
    let fp = match state.store.get_full_plan(plan_id, tenant_id, filter).await {
        Ok(fp) => fp,
        Err(error) if matches!(error.current_context(), StoreError::ValueNotFound(_)) => {
            return Err(match version {
                Some(number) => Problem::field(
                    format!("{path}/version"),
                    "not_found",
                    format!("The plan has no version {number}"),
                ),
                None => Problem::field(
                    format!("{path}/plan"),
                    "not_found",
                    "The plan has no published version",
                ),
            });
        }
        Err(error) => return Err(error.into()),
    };
    if fp.version.is_draft_version {
        return Err(Problem::field(
            format!("{path}/version"),
            "invalid",
            format!(
                "Version {} is a draft: a subscription buys a published version",
                fp.version.version
            ),
        ));
    }
    Ok(fp)
}

/// The option of the version an alias names, refused at `path` naming the version's options.
pub fn option_named<'a>(
    fp: &'a FullPlan,
    alias: &str,
    path: &str,
) -> Result<&'a PlanOption, Problem> {
    let options = &fp.version.options;
    options
        .iter()
        .find(|option| option.alias.eq_ignore_ascii_case(alias.trim()))
        .ok_or_else(|| {
            let known: Vec<&str> = options.iter().map(|o| o.alias.as_str()).collect();
            Problem::field(
                path,
                "not_found",
                format!(
                    "The plan has no option '{}': its options are {}",
                    alias.trim(),
                    if known.is_empty() {
                        "none".to_owned()
                    } else {
                        known.join(", ")
                    }
                ),
            )
        })
}

/// The currency and option a new subscription of `fp` is sold in, as the store picks them
/// (`SubscriptionCurrency::for_new_subscription`): with an option named, the first of the
/// customer's currency, the plan's and the others the option sells; else the first of them some
/// option sells, with its first such option.
pub fn sale(
    customer_currency: &str,
    fp: &FullPlan,
    named: Option<&PlanOption>,
) -> (String, Option<PlanOption>) {
    let options = &fp.version.options;
    let components = &fp.price_components;
    let plan_currency = fp.version.currency.trim().to_ascii_uppercase();
    if components.iter().all(|c| c.prices.is_empty()) || options.is_empty() {
        return (plan_currency, named.or_else(|| options.first()).cloned());
    }
    // A pre-currency price is in the plan's currency only.
    let legacy = components
        .iter()
        .any(|c| c.prices.is_empty() && c.legacy_pricing.is_some());
    let mut candidates = vec![customer_currency.to_owned(), plan_currency.clone()];
    candidates.extend(currencies(components));
    let candidates: Vec<String> = candidates
        .into_iter()
        .map(|c| c.trim().to_ascii_uppercase())
        .filter(|c| !legacy || c.eq_ignore_ascii_case(&plan_currency))
        .collect();
    if let Some(option) = named {
        let currency = candidates
            .iter()
            .find(|c| OptionPricing::new(option.id, c).sells(components))
            .cloned()
            .unwrap_or(plan_currency);
        return (currency, Some(option.clone()));
    }
    for currency in &candidates {
        if let Some(option) = sold_options(options, components, currency).first() {
            return (currency.clone(), Some((*option).clone()));
        }
    }
    (plan_currency, options.first().cloned())
}

/// The version's charges as one buying `option` in `currency` gets them: each its cell only.
pub fn cells(fp: &FullPlan, option: Option<&PlanOption>, currency: &str) -> Vec<PriceComponent> {
    let mut cells = fp.price_components.clone();
    if let Some(option) = option {
        OptionPricing::new(option.id, currency).narrow(&mut cells);
    }
    cells
}

/// The product code of `product_id`, as the plan knows it.
fn product_code(fp: &FullPlan, product_id: ProductId) -> Option<&str> {
    fp.products.get(&product_id).map(|p| p.code.as_str())
}

/// The charge of `components` billing the product `raw` names (by code or id), refused at `path`.
pub fn plan_charge<'a>(
    fp: &FullPlan,
    components: &'a [PriceComponent],
    raw: &str,
    path: &str,
) -> Result<&'a PriceComponent, Problem> {
    let raw = raw.trim();
    let found: Vec<&PriceComponent> = components
        .iter()
        .filter(|component| {
            component.product_id.is_some_and(|product_id| {
                product_code(fp, product_id) == Some(raw)
                    || product_id.to_string() == raw
                    || product_id.as_uuid().to_string() == raw
            })
        })
        .collect();
    match found.as_slice() {
        [component] => Ok(component),
        [] => Err(Problem::field(
            path,
            "not_found",
            format!("The plan has no charge of the product {raw}"),
        )),
        _ => Err(Problem::field(
            path,
            "ambiguous",
            format!("Several charges of the plan bill the product {raw}"),
        )),
    }
}

/// Whether a seat charge of `structure` counts `quantity`.
pub fn counts(structure: Option<&FeeStructure>, quantity: &ManagedQuantity) -> bool {
    match structure {
        Some(FeeStructure::Slot {
            quantity_id: Some(id),
            ..
        }) => *id == quantity.id,
        // Stored before quantities were named: its unit is the quantity's code.
        Some(FeeStructure::Slot {
            quantity_id: None,
            unit_name,
            ..
        }) => *unit_name == quantity.code,
        _ => false,
    }
}

fn decimal(raw: &str, path: &str) -> Result<Decimal, Problem> {
    let raw = raw.trim();
    let unsigned = raw.strip_prefix('+').unwrap_or(raw);
    Decimal::from_str(unsigned)
        .map_err(|_| Problem::field(path, "invalid", format!("'{raw}' is not a decimal number")))
}

/// A whole count of units, 0 or more.
pub fn count(raw: &str, path: &str) -> Result<u64, Problem> {
    let value = decimal(raw, path)?.normalize();
    if value.is_sign_negative() || value.scale() > 0 {
        return Err(Problem::field(
            path,
            "invalid",
            format!("'{}' is not a whole number of units", raw.trim()),
        ));
    }
    u64::try_from(value.mantissa())
        .map_err(|_| Problem::field(path, "invalid", "The number is too large"))
}

/// A whole number of seats, as many as the store holds (`i32::MAX`).
pub fn seats(raw: &str, path: &str) -> Result<u32, Problem> {
    let count = count(raw, path)?;
    if count > i32::MAX as u64 {
        return Err(Problem::field(
            path,
            "invalid",
            format!("{} seats at most", i32::MAX),
        ));
    }
    u32::try_from(count).map_err(|_| Problem::field(path, "invalid", "The number is too large"))
}

/// A whole change of seats, signed: `"+1"`, `"-2"`.
pub fn seat_delta(raw: &str, path: &str) -> Result<i32, Problem> {
    let value = decimal(raw, path)?.normalize();
    if value.scale() > 0 {
        return Err(Problem::field(
            path,
            "invalid",
            format!("'{}' is not a whole number of seats", raw.trim()),
        ));
    }
    let magnitude = i32::try_from(value.mantissa())
        .map_err(|_| Problem::field(path, "invalid", "The number is too large"))?;
    if magnitude == 0 {
        return Err(Problem::field(
            path,
            "invalid",
            "A change of 0 seats changes nothing",
        ));
    }
    Ok(magnitude)
}

/// `problem` with the errors at `from…` moved to `to…`: the price a request sends alone, read as
/// the first price of the charge it is checked in.
fn repath(mut problem: Problem, from: &str, to: &str) -> Problem {
    for error in &mut problem.errors {
        if let Some(rest) = error.path.strip_prefix(from) {
            error.path = format!("{to}{rest}");
        }
    }
    problem
}

/// A stored plan charge as `charges::render` reads it, its prices those of `component`.
pub fn plan_stored(component: &PriceComponent, path: &str) -> Result<StoredCharge, Problem> {
    let product_id = component.product_id.ok_or_else(|| {
        Problem::not_expressible(path, "a charge without a product: change it with API v1")
    })?;
    Ok(StoredCharge {
        name: component.name.clone(),
        product_id,
        structure: component.fee_structure.clone(),
        prices: component
            .prices
            .iter()
            .map(|price| StoredPrice {
                cadence: price.cadence,
                currency: price.currency.clone(),
                pricing: price.pricing.clone(),
                credit_unit: price.credit_unit.clone(),
                options: OptionSet::Every,
            })
            .collect(),
    })
}

/// The price an override asks of the charge `stored` (billed as it bills, in `currency`):
/// refused when the price would bill the charge differently. `path` is the override's.
pub async fn override_price(
    state: &AppState,
    tenant_id: TenantId,
    stored: &StoredCharge,
    asked: &ChargeOverride,
    cadence: Cadence,
    currency: &str,
    path: &str,
) -> Result<PriceInput, Problem> {
    let Some(structure) = &stored.structure else {
        return Err(Problem::not_expressible(
            format!("{path}/charge"),
            "this charge includes its product for free: it has no price to override",
        ));
    };
    let names = CatalogNames::load(state, tenant_id, [stored]).await?;
    let mut charge = render(stored, &[], &names, &format!("{path}/charge"))?;
    let first = charge.prices.first().cloned();
    charge.preset = None;
    charge.prices = vec![ChargePrice {
        option: EVERY_OPTION.to_owned(),
        currency: currency.to_owned(),
        cadence,
        model: asked.price.clone(),
        credit_unit: first.as_ref().and_then(|p| p.credit_unit.clone()),
        grants: first.and_then(|p| p.grants),
    }];
    // An allowance is a per-unit price's: a model of its own replaces it.
    if !matches!(asked.price, PriceModel::PerUnit(_)) {
        charge.included = None;
    }
    let refs = resolve_refs(state, tenant_id, &charge, path).await?;
    let domain = to_domain(&charge, &refs, path).map_err(|problem| {
        repath(
            problem,
            &format!("{path}/prices/0"),
            &format!("{path}/price"),
        )
    })?;
    let same = domain
        .structure
        .as_ref()
        .is_some_and(|asked| asked.same_shape(structure));
    if !same {
        return Err(Problem::not_expressible(
            format!("{path}/price"),
            "this price bills the charge differently than it does: a charge keeps how it bills \
             its product, only its price changes",
        ));
    }
    let mut prices = domain.prices.into_iter().map(|(price, _)| price);
    match (prices.next(), prices.next()) {
        (Some(price), None) => Ok(price),
        _ => Err(Problem::not_expressible(
            format!("{path}/price"),
            "a subscription's charge bills one price: one level, for usage sold in levels",
        )),
    }
}

/// A charge a subscription adds beside its plan's: its name, product, structure and its one price
/// in the subscription's `currency`.
pub async fn own_charge(
    state: &AppState,
    tenant_id: TenantId,
    charge: &Charge,
    currency: &str,
    path: &str,
) -> Result<(String, ProductId, FeeStructure, PriceInput), Problem> {
    for (index, price) in charge.prices.iter().enumerate() {
        if price.option.trim() != EVERY_OPTION {
            return Err(Problem::not_expressible(
                format!("{path}/prices/{index}/option"),
                "a subscription has no options: its own charges' prices are `*`",
            ));
        }
    }
    let refs = resolve_refs(state, tenant_id, charge, path).await?;
    let domain = to_domain(charge, &refs, path)?;
    let Some(structure) = domain.structure else {
        return Err(Problem::field(
            format!("{path}/quantity"),
            "required",
            "A charge added to a subscription is priced: send its quantity or a preset, and \
             its price",
        ));
    };
    let mut prices = domain
        .prices
        .into_iter()
        .map(|(price, _)| price)
        .filter(|price| price.currency.eq_ignore_ascii_case(currency));
    match (prices.next(), prices.next()) {
        (Some(price), None) => Ok((domain.name, domain.product_id, structure, price)),
        (None, _) => Err(Problem::field(
            format!("{path}/prices"),
            "required",
            format!("A price in {currency}, the subscription's currency"),
        )),
        (Some(_), Some(_)) => Err(Problem::not_expressible(
            format!("{path}/prices"),
            format!(
                "one price in {currency}: a subscription's charge bills one cadence, and usage \
                 sold in levels one level"
            ),
        )),
    }
}

/// A refusal of the request as a conflict with the subscription's state.
pub fn conflict(message: impl Into<String>) -> Problem {
    Problem::new(StatusCode::CONFLICT, ProblemCode::Conflict, message)
}
