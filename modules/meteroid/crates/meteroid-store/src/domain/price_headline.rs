//! A plan option's price in one currency, as a pricing page card and the plans list read it:
//! its fixed fees on one period, what comes on top beside, in the order its card reads them.
use std::collections::HashMap;

use super::Price;
use super::enums::BillingPeriodEnum;
use super::plan_options::PlanOption;
use super::plans::PricedOption;
use super::price_cells::{OptionPricing, priced_options};
use super::price_components::{
    PriceComponent, RatingModel, TierRow, UsagePricingModel, fixed_quantity_amount,
};
use super::prices::{FeeStructure, Pricing};
use super::pricing_pages::{CardExtra, CardExtraKind, CardLevel, CardLevels, money};
use common_domain::ids::{BillableMetricId, ProductId};
use diesel_models::billable_metrics::BillableMetricRow;
use rust_decimal::Decimal;

/// A fee recurring on its price's cadence; usage and one-time charges have none of their own.
fn recurs(pricing: &Pricing) -> bool {
    !matches!(pricing, Pricing::Usage(_) | Pricing::OneTime { .. })
}

/// What the charges' products bill and their metrics count; empty, extras name no unit.
#[derive(Default)]
pub(crate) struct Catalog {
    pub(crate) products: HashMap<ProductId, FeeStructure>,
    pub(crate) metrics: HashMap<BillableMetricId, BillableMetricRow>,
}

impl Catalog {
    fn fee_structure(&self, component: &PriceComponent) -> Option<&FeeStructure> {
        self.products.get(&component.product_id?)
    }

    /// The metric a usage charge counts.
    fn metric(&self, component: &PriceComponent) -> Option<&BillableMetricRow> {
        let metric_id = match self.fee_structure(component)? {
            FeeStructure::Usage { metric_id, .. } | FeeStructure::Capacity { metric_id } => {
                metric_id
            }
            _ => return None,
        };
        self.metrics.get(metric_id)
    }

    /// What a usage charge counts, in the metric's words.
    fn metric_unit(&self, component: &PriceComponent) -> Option<String> {
        let metric = self.metric(component)?;
        Some(
            metric
                .unit_singular
                .clone()
                .filter(|unit| !unit.trim().is_empty())
                .unwrap_or_else(|| metric.name.clone()),
        )
    }
}

/// What a plan's charges amount to in one option, all read from its cells in one currency.
#[derive(Default)]
pub(crate) struct Priced {
    pub(crate) fixed: Decimal,
    /// The products of the fees `fixed` sums: the card's base price.
    pub(crate) base: Vec<ProductId>,
    pub(crate) extras: Vec<CardExtra>,
    pub(crate) from_lowest: bool,
    pub(crate) partial: bool,
    /// The levels of its one usage-levels charge, each `amount` the card's total with it.
    pub(crate) levels: Option<CardLevels>,
}

impl Priced {
    /// A fee billed on the card's period, in its base price.
    fn add_base(&mut self, component: &PriceComponent, amount: Decimal) {
        self.fixed += amount;
        if let Some(id) = component.product_id
            && !self.base.contains(&id)
        {
            self.base.push(id);
        }
    }
}

/// An extra of `component`'s charge, nothing read yet.
fn extra(kind: CardExtraKind, component: &PriceComponent) -> CardExtra {
    CardExtra {
        kind,
        name: component.name.clone(),
        amount: None,
        from_lowest: false,
        included: None,
        block_size: None,
        unit: None,
        period: None,
        product_ids: component.product_id.into_iter().collect(),
        can_lead: false,
        lead: false,
    }
}

/// A seat or usage price as one price per unit, when it reads as one: per unit ("$12 / seat"); a
/// free first tier then one rate, graduated ("$12 / seat", 3 included); other tiers from their
/// lowest rate ("from $8 / seat"); a package ("$30 / 10 units"). A rate card, or rates by
/// dimension, have none.
#[derive(Default)]
struct UnitPrice {
    amount: Option<Decimal>,
    from_lowest: bool,
    included: Option<u64>,
    block_size: Option<u64>,
}

impl UnitPrice {
    fn per_unit(rate: Decimal) -> Self {
        UnitPrice {
            amount: Some(rate),
            ..Default::default()
        }
    }

    fn package(block_size: u64, rate: Decimal) -> Self {
        UnitPrice {
            amount: Some(rate),
            block_size: (block_size > 1).then_some(block_size),
            ..Default::default()
        }
    }

    /// `graduated`: each unit at its tier's rate, a free first tier then an allowance; by volume
    /// every unit is at the rate of the tier reached, so it never is.
    fn tiers(tiers: &[TierRow], graduated: bool, block_size: Option<u64>) -> Self {
        let block_size = block_size.filter(|size| *size > 1);
        let no_fee = |tier: &TierRow| tier.flat_fee.is_none_or(|fee| fee.is_zero());
        if graduated
            && let [free, paid] = tiers
            && free.first_unit == 0
            && free.rate.is_zero()
            && no_fee(free)
            && no_fee(paid)
            && paid.rate > Decimal::ZERO
        {
            return UnitPrice {
                amount: Some(paid.rate),
                included: Some(paid.first_unit),
                block_size,
                ..Default::default()
            };
        }
        let lowest = tiers
            .iter()
            .map(|tier| tier.rate)
            .filter(|rate| *rate > Decimal::ZERO)
            .min();
        UnitPrice {
            amount: lowest,
            from_lowest: lowest.is_some() && tiers.len() > 1,
            included: None,
            block_size,
        }
    }

    fn of_rating(model: &RatingModel) -> Self {
        match model {
            RatingModel::PerUnit { rate } => Self::per_unit(*rate),
            RatingModel::Tiered { tiers } => Self::tiers(tiers, true, None),
            RatingModel::Volume { tiers } => Self::tiers(tiers, false, None),
            RatingModel::Package { block_size, rate } => Self::package(*block_size, *rate),
        }
    }

    fn of_usage(model: &UsagePricingModel) -> Self {
        match model {
            UsagePricingModel::PerUnit { rate } => Self::per_unit(*rate),
            UsagePricingModel::Tiered { tiers, block_size } => {
                Self::tiers(tiers, true, *block_size)
            }
            UsagePricingModel::Volume { tiers, block_size } => {
                Self::tiers(tiers, false, *block_size)
            }
            UsagePricingModel::Package { block_size, rate } => Self::package(*block_size, *rate),
            // A rate card's rates live on the card; rates by dimension are several.
            _ => Self::default(),
        }
    }

    /// `extra` reading this price: it can lead the card when it has one.
    fn on(self, extra: CardExtra) -> CardExtra {
        CardExtra {
            can_lead: self.amount.is_some(),
            amount: self.amount,
            from_lowest: self.from_lowest,
            included: self.included,
            block_size: self.block_size,
            ..extra
        }
    }
}

/// `components` already narrowed to one option's cells in one currency; `term` the card's period,
/// a fee billed on another shown beside its amount. Extras as charged; `in_order` orders them.
pub(crate) fn priced(
    components: &[PriceComponent],
    term: Option<BillingPeriodEnum>,
    catalog: &Catalog,
) -> Priced {
    let mut out = Priced::default();
    // Each usage-levels charge with its lowest fee, the one `fixed` holds.
    let mut offered: Vec<(CardLevels, Decimal)> = Vec::new();
    for component in components {
        let prices = &component.prices;
        if prices.is_empty() {
            // Included only, or legacy pricing no cell prices: the latter is never shown free.
            out.partial |= component.legacy_pricing.is_some();
            continue;
        }
        if let [price] = prices.as_slice() {
            add_price(&mut out, component, price, term, catalog);
            continue;
        }
        // Several prices in one cell: capacity levels show from their lowest; anything else
        // can't be told apart here and leaves the card partial.
        let levels: Vec<(Decimal, Decimal)> = prices
            .iter()
            .filter_map(|p| match &p.pricing {
                Pricing::Capacity {
                    rate, overage_rate, ..
                } => Some((*rate, *overage_rate)),
                _ => None,
            })
            .collect();
        if levels.len() != prices.len() {
            out.partial = true;
            continue;
        }
        let lowest = levels
            .iter()
            .map(|(rate, _)| *rate)
            .min()
            .unwrap_or_default();
        let overage = levels[0].1;
        out.add_base(component, lowest);
        out.from_lowest = true;
        // Its fee is in the base price; its overage never leads, the level is what it sells.
        out.extras.push(CardExtra {
            amount: levels
                .iter()
                .all(|(_, rate)| *rate == overage)
                .then_some(overage),
            unit: catalog.metric_unit(component),
            ..extra(CardExtraKind::Usage, component)
        });
        offered.push((card_levels(component, catalog), lowest));
    }
    // One charge's levels are offered, each the card's total with its fee in place of the
    // lowest; several leave the card reading "from" their lowest.
    if offered.len() == 1
        && let Some((mut levels, lowest)) = offered.pop()
    {
        for level in &mut levels.levels {
            level.amount = out.fixed - lowest + level.amount;
        }
        out.levels = Some(levels);
    }
    out
}

/// A usage-levels charge's levels by the usage they include, each `amount` its fee alone.
fn card_levels(component: &PriceComponent, catalog: &Catalog) -> CardLevels {
    let mut levels: Vec<CardLevel> = component
        .prices
        .iter()
        .filter_map(|price| match &price.pricing {
            Pricing::Capacity {
                rate,
                included,
                overage_rate,
            } => Some(CardLevel {
                included: *included,
                amount: *rate,
                monthly_equivalent: None,
                // Charged in a credit unit, not the card's currency.
                overage_rate: price.credit_unit.is_none().then_some(*overage_rate),
            }),
            _ => None,
        })
        .collect();
    levels.sort_by(|a, b| a.included.cmp(&b.included).then(a.amount.cmp(&b.amount)));
    let metric = catalog.metric(component);
    let named = |unit: &Option<String>| unit.clone().filter(|unit| !unit.trim().is_empty());
    CardLevels {
        name: component.name.clone(),
        levels,
        metric_name: metric.map(|metric| metric.name.clone()),
        unit_singular: metric.and_then(|metric| named(&metric.unit_singular)),
        unit_plural: metric.and_then(|metric| named(&metric.unit_plural)),
        unit_conversion_factor: metric.and_then(|metric| metric.unit_conversion_factor),
        more: None,
        product_id: component.product_id,
    }
}

fn add_price(
    out: &mut Priced,
    component: &PriceComponent,
    price: &Price,
    term: Option<BillingPeriodEnum>,
    catalog: &Catalog,
) {
    // A fee billed on another period than the card's never joins its amount.
    let own_period = term
        .filter(|term| *term != price.cadence)
        .map(|_| price.cadence);
    let fixed_fee = |out: &mut Priced, amount: Decimal| match own_period {
        Some(period) => out.extras.push(CardExtra {
            amount: Some(amount),
            period: Some(period),
            ..extra(CardExtraKind::Recurring, component)
        }),
        None => out.add_base(component, amount),
    };
    match &price.pricing {
        Pricing::Rate { rate } => fixed_fee(out, *rate),
        Pricing::ExtraRecurring {
            unit_price,
            quantity,
            model,
        } => fixed_fee(
            out,
            fixed_quantity_amount(*unit_price, Decimal::from(*quantity), model.as_ref()),
        ),
        Pricing::Capacity {
            rate,
            included,
            overage_rate,
        } => {
            // Its fee is in the base price; its overage never leads, the level is what it sells.
            out.add_base(component, *rate);
            out.extras.push(CardExtra {
                amount: Some(*overage_rate),
                included: Some(*included),
                unit: catalog.metric_unit(component),
                ..extra(CardExtraKind::Usage, component)
            });
        }
        Pricing::Slot { model, .. } => out.extras.push(UnitPrice::of_rating(model).on(CardExtra {
            unit: match catalog.fee_structure(component) {
                Some(FeeStructure::Slot { unit_name, .. }) => Some(unit_name.clone()),
                _ => None,
            },
            period: term,
            ..extra(CardExtraKind::PerSeat, component)
        })),
        Pricing::Usage(model) => out.extras.push(UnitPrice::of_usage(model).on(CardExtra {
            unit: catalog.metric_unit(component),
            ..extra(CardExtraKind::Usage, component)
        })),
        Pricing::OneTime {
            unit_price,
            quantity,
            model,
        } => out.extras.push(CardExtra {
            amount: Some(fixed_quantity_amount(
                *unit_price,
                Decimal::from(*quantity),
                model.as_ref(),
            )),
            ..extra(CardExtraKind::OneTime, component)
        }),
        // A pack sold for a price; credit granted at no charge is included, not a charge.
        Pricing::Credit {
            unit_price,
            quantity,
            ..
        } => {
            let amount = *unit_price * Decimal::from(*quantity);
            if !amount.is_zero() {
                out.extras.push(CardExtra {
                    amount: Some(amount),
                    period: term,
                    ..extra(CardExtraKind::CreditPack, component)
                });
            }
        }
    }
}

/// Where an item reads when the operator has not placed it: the base price, fees billed on
/// another period, seats, usage, one-time fees, then credit packs.
fn default_rank(kind: CardExtraKind) -> u8 {
    match kind {
        CardExtraKind::Base => 0,
        CardExtraKind::Recurring => 1,
        CardExtraKind::PerSeat => 2,
        CardExtraKind::Usage => 3,
        CardExtraKind::OneTime => 4,
        CardExtraKind::CreditPack => 5,
    }
}

/// `priced` as a plan's card reads it: a list of items in the operator's `order` by product
/// (`PlanPricingPage`), the first that can lead being the card's price, the others reading below
/// it in that order.
///
/// The items: the base price (the fixed fees billed on the card's period, summed) at the
/// earliest place of its products, then each other charge. Products not in `order` follow the
/// ordered ones as computed (`default_rank`, each kind as charged); one of `order` the plan
/// doesn't charge is skipped.
///
/// The base price leads when not zero; a seat or usage when it reads as one price per unit
/// (`UnitPrice`); fees on another period, one-time fees, packs and a capacity's overage never.
/// The first item that can lead leads; any other whose products are all `hidden` is left off,
/// whatever it charges: the page is the operator's, who may tell that price elsewhere.
pub(crate) fn in_order(mut priced: Priced, order: &[ProductId], hidden: &[ProductId]) -> Priced {
    let mut items = std::mem::take(&mut priced.extras);
    // A zero base price reads only when levels move it.
    if !priced.fixed.is_zero() || priced.levels.is_some() {
        items.insert(
            0,
            CardExtra {
                kind: CardExtraKind::Base,
                name: String::new(),
                amount: Some(priced.fixed),
                from_lowest: false,
                included: None,
                block_size: None,
                unit: None,
                // The card's own.
                period: None,
                product_ids: priced.base.clone(),
                can_lead: true,
                lead: false,
            },
        );
    }
    let place = |item: &CardExtra| {
        item.product_ids
            .iter()
            .filter_map(|id| order.iter().position(|ordered| ordered == id))
            .min()
    };
    // Stable: items alike read as charged.
    items.sort_by_key(|item| (place(item).unwrap_or(usize::MAX), default_rank(item.kind)));

    // The price leading the card always reads; any other the operator left off is gone, its
    // price told elsewhere (the plan's highlights, say).
    if let Some(lead) = items.iter_mut().find(|item| item.can_lead) {
        lead.lead = true;
    }
    items.retain(|item| {
        item.lead
            || item.product_ids.is_empty()
            || !item.product_ids.iter().all(|id| hidden.contains(id))
    });
    priced.extras = items;
    // Levels left off still offer their fees, without their overage.
    if let Some(levels) = priced.levels.as_mut()
        && levels.product_id.is_some_and(|id| hidden.contains(&id))
    {
        for level in &mut levels.levels {
            level.overage_rate = None;
        }
    }
    priced
}

/// `components` narrowed to `pricing`'s cells.
pub(crate) fn narrowed(
    components: &[PriceComponent],
    pricing: &OptionPricing,
) -> Vec<PriceComponent> {
    let mut components = components.to_vec();
    pricing.narrow(&mut components);
    components
}

/// The period a card reads: that of the plan's own fee (a rate, or a capacity's) costing the most
/// per month, else of the fixed fee costing the most (an add-on billed quarterly beside a monthly
/// platform fee reads beside it); none for a plan billing only usage or one-time charges.
pub(crate) fn card_period(components: &[PriceComponent]) -> Option<BillingPeriodEnum> {
    let prices = || components.iter().flat_map(|c| c.prices.iter());
    let per_month = |price: &Price| {
        let amount = match &price.pricing {
            Pricing::Rate { rate } | Pricing::Capacity { rate, .. } => *rate,
            Pricing::ExtraRecurring {
                unit_price,
                quantity,
                model,
            } => fixed_quantity_amount(*unit_price, Decimal::from(*quantity), model.as_ref()),
            _ => return None,
        };
        Some(amount / Decimal::from(price.cadence.as_months().max(1)))
    };
    let costliest = |own_fee: bool| {
        prices()
            .filter(|price| {
                !own_fee
                    || matches!(
                        price.pricing,
                        Pricing::Rate { .. } | Pricing::Capacity { .. }
                    )
            })
            .filter_map(|price| per_month(price).map(|monthly| (monthly, price.cadence)))
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
            .map(|(_, cadence)| cadence)
    };
    costliest(true).or_else(|| costliest(false)).or_else(|| {
        prices()
            .filter(|price| recurs(&price.pricing))
            .map(|price| price.cadence)
            .min()
    })
}

/// Every `(currency, option)` sold (`price_cells::priced_options`), each with its headline price.
pub(crate) fn headline_prices(
    options: &[PlanOption],
    components: &[PriceComponent],
) -> Vec<PricedOption> {
    // Units are only named on extras, which the headline doesn't show.
    let catalog = Catalog::default();
    priced_options(options, components)
        .into_iter()
        .map(|(currency, option_id)| {
            let cells = narrowed(components, &OptionPricing::new(option_id, &currency));
            let period = card_period(&cells);
            let on_period = priced(&cells, period, &catalog);
            let precision = crate::constants::Currencies::resolve_currency_precision(&currency);
            let readable = !on_period.partial;
            // Zero beside seats or usage isn't its price: it reads as free only when nothing is,
            // and as its seat price when seats are what it charges.
            let fixed_reads = !on_period.fixed.is_zero() || on_period.extras.is_empty();
            let seat = (readable && !fixed_reads)
                .then(|| {
                    // The plans list reads a seat's price only when every seat costs it.
                    on_period.extras.iter().find(|extra| {
                        extra.kind == CardExtraKind::PerSeat
                            && extra.amount.is_some()
                            && !extra.from_lowest
                            && extra.included.is_none()
                            && extra.block_size.is_none()
                    })
                })
                .flatten();
            PricedOption {
                amount: match seat {
                    Some(seat) => seat.amount.map(|amount| money(amount, precision)),
                    None => (readable && period.is_some() && fixed_reads)
                        .then(|| money(on_period.fixed, precision)),
                },
                unit: seat.map(|seat| seat.unit.clone().unwrap_or_else(|| "seat".to_owned())),
                period: period.filter(|_| readable),
                has_usage: readable
                    && on_period
                        .extras
                        .iter()
                        .any(|extra| extra.kind == CardExtraKind::Usage),
                from_lowest: readable && on_period.from_lowest,
                currency,
                option_id,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::price_components::OptionPrice;
    use common_domain::ids::{
        BaseId, PlanOptionId, PlanVersionId, PriceComponentId, PriceId, TenantId,
    };

    fn price(cadence: BillingPeriodEnum, pricing: Pricing) -> Price {
        Price {
            id: PriceId::new(),
            product_id: ProductId::default(),
            cadence,
            currency: "EUR".to_owned(),
            pricing,
            tenant_id: TenantId::default(),
            created_at: chrono::NaiveDateTime::default(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    fn option(name: &str, position: i32) -> PlanOption {
        PlanOption {
            id: PlanOptionId::new(),
            plan_version_id: PlanVersionId::default(),
            name: name.to_owned(),
            alias: name.to_ascii_lowercase(),
            position,
            hidden: false,
        }
    }

    fn component(cells: Vec<(Vec<PlanOptionId>, Price)>) -> PriceComponent {
        PriceComponent {
            id: PriceComponentId::new(),
            name: "charge".to_owned(),
            product_id: Some(ProductId::default()),
            option_prices: cells
                .iter()
                .flat_map(|(options, price)| {
                    options.iter().map(move |option| OptionPrice {
                        option_id: *option,
                        price_id: price.id,
                    })
                })
                .collect(),
            prices: cells.into_iter().map(|(_, price)| price).collect(),
            legacy_pricing: None,
        }
    }

    #[test]
    fn each_option_reads_its_fixed_fees_with_usage_on_top() {
        let (monthly, annual) = (option("Monthly", 0), option("Annual", 1));
        let rate = |rate: i64| Pricing::Rate {
            rate: Decimal::from(rate),
        };
        let platform = component(vec![
            (
                vec![monthly.id],
                price(BillingPeriodEnum::Monthly, rate(29)),
            ),
            (vec![annual.id], price(BillingPeriodEnum::Annual, rate(290))),
        ]);
        let api_calls = component(vec![(
            vec![monthly.id, annual.id],
            price(
                BillingPeriodEnum::Monthly,
                Pricing::Usage(UsagePricingModel::PerUnit { rate: Decimal::ONE }),
            ),
        )]);
        let options = [monthly.clone(), annual.clone()];
        let headlines = headline_prices(&options, &[platform, api_calls]);
        let read = |priced: &PricedOption| {
            (
                priced.option_id,
                priced.amount.map(|a| a.to_string()),
                priced.period,
                priced.has_usage,
            )
        };
        assert_eq!(
            headlines.iter().map(read).collect::<Vec<_>>(),
            vec![
                (
                    monthly.id,
                    Some("29.00".to_owned()),
                    Some(BillingPeriodEnum::Monthly),
                    true
                ),
                (
                    annual.id,
                    Some("290.00".to_owned()),
                    Some(BillingPeriodEnum::Annual),
                    true
                ),
            ]
        );
    }

    #[test]
    fn usage_only_has_no_amount() {
        let only = option("Standard", 0);
        let usage = component(vec![(
            vec![only.id],
            price(
                BillingPeriodEnum::Monthly,
                Pricing::Usage(UsagePricingModel::PerUnit { rate: Decimal::ONE }),
            ),
        )]);
        let headlines = headline_prices(std::slice::from_ref(&only), &[usage]);
        assert_eq!(headlines.len(), 1);
        assert_eq!(headlines[0].amount, None);
        assert_eq!(headlines[0].period, None);
        assert!(headlines[0].has_usage);
    }

    fn item(kind: CardExtraKind, product: ProductId, amount: Option<i64>) -> CardExtra {
        CardExtra {
            kind,
            name: format!("{kind:?}"),
            amount: amount.map(Decimal::from),
            from_lowest: false,
            included: None,
            block_size: None,
            unit: None,
            period: None,
            product_ids: vec![product],
            can_lead: amount.is_some()
                && matches!(kind, CardExtraKind::PerSeat | CardExtraKind::Usage),
            lead: false,
        }
    }

    fn card(fixed: i64, base: &[ProductId], extras: Vec<CardExtra>) -> Priced {
        Priced {
            fixed: Decimal::from(fixed),
            base: base.to_vec(),
            extras,
            from_lowest: false,
            partial: false,
            levels: None,
        }
    }

    /// Each item as `(kind, products, lead)`; a folded "+ usage" line lists the usage it folds.
    fn read(priced: &Priced) -> Vec<(CardExtraKind, Vec<ProductId>, bool)> {
        priced
            .extras
            .iter()
            .map(|extra| (extra.kind, extra.product_ids.clone(), extra.lead))
            .collect()
    }

    #[test]
    fn nothing_ordered_reads_as_computed() {
        let (platform, seats, calls) = (ProductId::new(), ProductId::new(), ProductId::new());
        let fixed = in_order(
            card(
                49,
                &[platform],
                vec![
                    item(CardExtraKind::Usage, calls, Some(1)),
                    item(CardExtraKind::PerSeat, seats, Some(12)),
                ],
            ),
            &[],
            &[],
        );
        assert_eq!(
            read(&fixed),
            vec![
                (CardExtraKind::Base, vec![platform], true),
                (CardExtraKind::PerSeat, vec![seats], false),
                (CardExtraKind::Usage, vec![calls], false),
            ],
            "the base price leads, then seats, then usage"
        );
        assert_eq!(fixed.extras[0].amount, Some(Decimal::from(49)));

        let nothing_fixed = in_order(
            card(
                0,
                &[],
                vec![
                    item(CardExtraKind::Usage, calls, Some(1)),
                    item(CardExtraKind::PerSeat, seats, Some(12)),
                ],
            ),
            &[],
            &[],
        );
        assert_eq!(
            read(&nothing_fixed),
            vec![
                (CardExtraKind::PerSeat, vec![seats], true),
                (CardExtraKind::Usage, vec![calls], false),
            ],
            "no base price item when nothing fixed is due"
        );

        let rate_card_only = in_order(
            card(0, &[], vec![item(CardExtraKind::Usage, calls, None)]),
            &[],
            &[],
        );
        assert!(
            !rate_card_only.extras[0].lead,
            "no single price to lead with"
        );
    }

    #[test]
    fn the_first_item_that_can_lead_is_the_price_the_others_read_in_order() {
        let (platform, seats, calls, onboarding, tokens) = (
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
        );
        let priced = || {
            card(
                49,
                &[platform],
                vec![
                    item(CardExtraKind::OneTime, onboarding, Some(500)),
                    item(CardExtraKind::Usage, tokens, None),
                    item(CardExtraKind::PerSeat, seats, Some(12)),
                    item(CardExtraKind::Usage, calls, Some(1)),
                ],
            )
        };
        assert_eq!(
            read(&in_order(priced(), &[calls, platform], &[])),
            vec![
                (CardExtraKind::Usage, vec![calls], true),
                (CardExtraKind::Base, vec![platform], false),
                (CardExtraKind::PerSeat, vec![seats], false),
                (CardExtraKind::Usage, vec![tokens], false),
                (CardExtraKind::OneTime, vec![onboarding], false),
            ],
            "the base price still reads, the products not ordered after, as computed"
        );
        assert_eq!(
            read(&in_order(priced(), &[onboarding, tokens, seats], &[])),
            vec![
                (CardExtraKind::OneTime, vec![onboarding], false),
                (CardExtraKind::Usage, vec![tokens], false),
                (CardExtraKind::PerSeat, vec![seats], true),
                (CardExtraKind::Base, vec![platform], false),
                (CardExtraKind::Usage, vec![calls], false),
            ],
            "a one-time fee and a rate card never lead: the seat after them does"
        );
        let absent = ProductId::new();
        assert_eq!(
            read(&in_order(priced(), &[absent, seats], &[]))[0],
            (CardExtraKind::PerSeat, vec![seats], true),
            "a product the version doesn't charge is skipped"
        );
    }

    #[test]
    fn the_base_price_reads_at_the_earliest_place_of_its_fees() {
        let (platform, support, seats) = (ProductId::new(), ProductId::new(), ProductId::new());
        let priced = in_order(
            card(
                149,
                &[platform, support],
                vec![item(CardExtraKind::PerSeat, seats, Some(12))],
            ),
            &[seats, support, platform],
            &[],
        );
        assert_eq!(
            read(&priced),
            vec![
                (CardExtraKind::PerSeat, vec![seats], true),
                (CardExtraKind::Base, vec![platform, support], false),
            ]
        );
    }

    #[test]
    fn whatever_is_hidden_is_left_off_but_the_price_leading() {
        let (platform, seats, calls, storage, egress, onboarding, pack) = (
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
            ProductId::new(),
        );
        let priced = in_order(
            card(
                49,
                &[platform],
                vec![
                    item(CardExtraKind::PerSeat, seats, Some(12)),
                    item(CardExtraKind::Usage, calls, Some(1)),
                    item(CardExtraKind::Usage, storage, Some(2)),
                    item(CardExtraKind::OneTime, onboarding, Some(500)),
                    item(CardExtraKind::Usage, egress, None),
                    item(CardExtraKind::CreditPack, pack, Some(100)),
                ],
            ),
            &[],
            &[platform, seats, storage, onboarding, egress, pack],
        );
        assert_eq!(
            read(&priced),
            vec![
                (CardExtraKind::Base, vec![platform], true),
                (CardExtraKind::Usage, vec![calls], false),
            ],
            "the base price leads, hidden or not; every other hidden charge is gone"
        );
    }

    #[test]
    fn the_price_leading_reads_even_hidden() {
        let (calls, storage) = (ProductId::new(), ProductId::new());
        let extras = || {
            vec![
                item(CardExtraKind::Usage, calls, Some(1)),
                item(CardExtraKind::Usage, storage, Some(2)),
            ]
        };
        assert_eq!(
            read(&in_order(card(0, &[], extras()), &[calls], &[calls])),
            vec![
                (CardExtraKind::Usage, vec![calls], true),
                (CardExtraKind::Usage, vec![storage], false),
            ],
            "ordered first, it leads, hidden or not"
        );
        assert_eq!(
            read(&in_order(card(0, &[], extras()), &[], &[calls, storage])),
            vec![(CardExtraKind::Usage, vec![calls], true)],
            "a card never loses its price"
        );
    }

    fn tier(first_unit: u64, rate: &str) -> TierRow {
        TierRow {
            first_unit,
            rate: rate.parse().unwrap(),
            flat_fee: None,
            flat_cap: None,
        }
    }

    #[test]
    fn a_seat_or_usage_price_reads_per_unit_whatever_its_model() {
        let read = |model: RatingModel| {
            let price = UnitPrice::of_rating(&model);
            (
                price.amount,
                price.from_lowest,
                price.included,
                price.block_size,
            )
        };
        assert_eq!(
            read(RatingModel::PerUnit {
                rate: Decimal::from(12)
            }),
            (Some(Decimal::from(12)), false, None, None)
        );
        assert_eq!(
            read(RatingModel::Tiered {
                tiers: vec![tier(0, "0"), tier(3, "12")]
            }),
            (Some(Decimal::from(12)), false, Some(3), None),
            "3 seats included, then $12 a seat"
        );
        assert_eq!(
            read(RatingModel::Volume {
                tiers: vec![tier(0, "0"), tier(3, "12")]
            }),
            (Some(Decimal::from(12)), true, None, None),
            "by volume, a free first tier is no allowance"
        );
        assert_eq!(
            read(RatingModel::Tiered {
                tiers: vec![tier(0, "12"), tier(10, "10"), tier(50, "8")]
            }),
            (Some(Decimal::from(8)), true, None, None),
            "from the lowest rate"
        );
        assert_eq!(
            read(RatingModel::Package {
                block_size: 10,
                rate: Decimal::from(30)
            }),
            (Some(Decimal::from(30)), false, None, Some(10))
        );
        let rate_card = UnitPrice::of_usage(&UsagePricingModel::RateCard {
            rate_card_id: common_domain::ids::RateCardId::new(),
            multiplier: None,
        });
        assert_eq!(rate_card.amount, None, "a rate card has no one price");
    }

    /// Pro: a $99 platform fee (monthly, $990 a year), 10 support hours at $60 a quarter, seats
    /// 3 free then $12, events 1M free then $0.0004, LLM tokens by rate card. Seats ordered first.
    #[test]
    fn the_pro_plan_led_by_its_tiered_seats() {
        let (monthly, annual) = (option("Monthly", 0), option("Annual", 1));
        let named = |name: &str, cells: Vec<(Vec<PlanOptionId>, Price)>| {
            let mut charge = component(cells);
            charge.name = name.to_owned();
            charge.product_id = Some(ProductId::new());
            charge
        };
        let both = vec![monthly.id, annual.id];
        let platform = named(
            "Platform fee",
            vec![
                (
                    vec![monthly.id],
                    price(
                        BillingPeriodEnum::Monthly,
                        Pricing::Rate {
                            rate: Decimal::from(99),
                        },
                    ),
                ),
                (
                    vec![annual.id],
                    price(
                        BillingPeriodEnum::Annual,
                        Pricing::Rate {
                            rate: Decimal::from(990),
                        },
                    ),
                ),
            ],
        );
        let support = named(
            "Support hours",
            vec![(
                both.clone(),
                price(
                    BillingPeriodEnum::Quarterly,
                    Pricing::ExtraRecurring {
                        unit_price: Decimal::from(60),
                        quantity: 10,
                        model: None,
                    },
                ),
            )],
        );
        let seats = named(
            "Seats",
            vec![(
                both.clone(),
                price(
                    BillingPeriodEnum::Monthly,
                    Pricing::Slot {
                        model: RatingModel::Tiered {
                            tiers: vec![tier(0, "0"), tier(3, "12")],
                        },
                        min_slots: None,
                        max_slots: None,
                    },
                ),
            )],
        );
        let events = named(
            "Events",
            vec![(
                both.clone(),
                price(
                    BillingPeriodEnum::Monthly,
                    Pricing::Usage(UsagePricingModel::Tiered {
                        tiers: vec![tier(0, "0"), tier(1_000_000, "0.0004")],
                        block_size: None,
                    }),
                ),
            )],
        );
        let tokens = named(
            "LLM tokens",
            vec![(
                both,
                price(
                    BillingPeriodEnum::Monthly,
                    Pricing::Usage(UsagePricingModel::RateCard {
                        rate_card_id: common_domain::ids::RateCardId::new(),
                        multiplier: None,
                    }),
                ),
            )],
        );
        let order = [seats.product_id.unwrap()];
        let components = [platform, support, seats, events, tokens];
        let cells = narrowed(&components, &OptionPricing::new(monthly.id, "EUR"));
        let period = card_period(&cells);
        assert_eq!(
            period,
            Some(BillingPeriodEnum::Monthly),
            "the platform fee's period, not the support hours' costing more a month"
        );
        let on_card = in_order(priced(&cells, period, &Catalog::default()), &order, &[]);
        let read: Vec<_> = on_card
            .extras
            .iter()
            .map(|extra| {
                (
                    extra.kind,
                    extra.name.as_str(),
                    extra.amount.map(|amount| amount.normalize().to_string()),
                    extra.included,
                    extra.period,
                    extra.lead,
                )
            })
            .collect();
        assert_eq!(
            read,
            vec![
                (
                    CardExtraKind::PerSeat,
                    "Seats",
                    Some("12".to_owned()),
                    Some(3),
                    Some(BillingPeriodEnum::Monthly),
                    true
                ),
                (
                    CardExtraKind::Base,
                    "",
                    Some("99".to_owned()),
                    None,
                    None,
                    false
                ),
                (
                    CardExtraKind::Recurring,
                    "Support hours",
                    Some("600".to_owned()),
                    None,
                    Some(BillingPeriodEnum::Quarterly),
                    false
                ),
                (
                    CardExtraKind::Usage,
                    "Events",
                    Some("0.0004".to_owned()),
                    Some(1_000_000),
                    None,
                    false
                ),
                (CardExtraKind::Usage, "LLM tokens", None, None, None, false),
            ],
            "$12 / seat, 3 seats included, + $99 / mo base price, support hours a quarter"
        );

        let cells = narrowed(&components, &OptionPricing::new(annual.id, "EUR"));
        let period = card_period(&cells);
        let on_card = in_order(priced(&cells, period, &Catalog::default()), &order, &[]);
        assert_eq!(period, Some(BillingPeriodEnum::Annual));
        assert_eq!(
            on_card.extras[0].product_ids,
            order.to_vec(),
            "the order follows the product into each option"
        );
        assert_eq!(on_card.extras[1].amount, Some(Decimal::from(990)));
    }

    fn level(rate: i64, included: u64) -> Price {
        price(
            BillingPeriodEnum::Monthly,
            Pricing::Capacity {
                rate: Decimal::from(rate),
                included,
                overage_rate: Decimal::new(5, 3),
            },
        )
    }

    /// Volume plans: a €29 platform fee, and €199 for €50K managed revenue, €479 for €125K,
    /// €899 for €250K, 0.5% above the level; the levels listed out of order.
    fn volume(only: &PlanOption) -> (PriceComponent, PriceComponent) {
        let mut platform = component(vec![(
            vec![only.id],
            price(
                BillingPeriodEnum::Monthly,
                Pricing::Rate {
                    rate: Decimal::from(29),
                },
            ),
        )]);
        platform.product_id = Some(ProductId::new());
        let mut revenue = component(vec![
            (vec![only.id], level(479, 125_000)),
            (vec![only.id], level(199, 50_000)),
            (vec![only.id], level(899, 250_000)),
        ]);
        revenue.product_id = Some(ProductId::new());
        revenue.name = "Managed revenue".to_owned();
        (platform, revenue)
    }

    fn read_levels(priced: &Priced) -> Vec<(u64, Decimal, Option<Decimal>)> {
        priced
            .levels
            .iter()
            .flat_map(|levels| levels.levels.iter())
            .map(|level| (level.included, level.amount, level.overage_rate))
            .collect()
    }

    #[test]
    fn a_card_offers_its_usage_levels_each_at_the_cards_total() {
        let only = option("Monthly", 0);
        let (platform, revenue) = volume(&only);
        let product = revenue.product_id.unwrap();
        let cells = narrowed(&[platform, revenue], &OptionPricing::new(only.id, "EUR"));
        let on_card = priced(&cells, card_period(&cells), &Catalog::default());
        assert_eq!(
            on_card.fixed,
            Decimal::from(29 + 199),
            "the lowest level in the amount"
        );
        assert!(on_card.from_lowest);
        let overage = Some(Decimal::new(5, 3));
        assert_eq!(
            read_levels(&on_card),
            vec![
                (50_000, Decimal::from(29 + 199), overage),
                (125_000, Decimal::from(29 + 479), overage),
                (250_000, Decimal::from(29 + 899), overage),
            ],
            "smallest first, each with the platform fee"
        );
        let levels = on_card.levels.as_ref().unwrap();
        assert_eq!(levels.name, "Managed revenue");
        assert_eq!(levels.product_id, Some(product));
        assert_eq!(levels.more, None, "the page decides where More leads");
        assert_eq!(on_card.base.len(), 2, "its fee is in the base price");
        let overage_items: Vec<_> = on_card
            .extras
            .iter()
            .filter(|extra| extra.product_ids.contains(&product))
            .collect();
        assert_eq!(
            overage_items.len(),
            1,
            "its overage still reads among the extras"
        );
        assert!(!overage_items[0].can_lead);
    }

    #[test]
    fn one_capacity_price_or_several_levels_charges_offer_no_levels() {
        let only = option("Monthly", 0);
        let single = component(vec![(vec![only.id], level(479, 125_000))]);
        let on_card = priced(
            &[single],
            Some(BillingPeriodEnum::Monthly),
            &Catalog::default(),
        );
        assert!(on_card.levels.is_none(), "nothing to pick");
        assert_eq!(on_card.fixed, Decimal::from(479));
        assert_eq!(on_card.extras[0].included, Some(125_000));

        let (_, revenue) = volume(&only);
        let mut storage = component(vec![
            (vec![only.id], level(10, 5)),
            (vec![only.id], level(40, 25)),
        ]);
        storage.product_id = Some(ProductId::new());
        let on_card = priced(
            &[revenue, storage],
            Some(BillingPeriodEnum::Monthly),
            &Catalog::default(),
        );
        assert!(on_card.levels.is_none(), "one slider per card");
        assert!(on_card.from_lowest, "the card reads from their lowest");
        assert_eq!(on_card.fixed, Decimal::from(199 + 10));
    }

    #[test]
    fn levels_keep_the_operators_order() {
        let only = option("Monthly", 0);
        let (platform, revenue) = volume(&only);
        let product = revenue.product_id.unwrap();
        let mut seats = component(vec![(
            vec![only.id],
            price(
                BillingPeriodEnum::Monthly,
                Pricing::Slot {
                    model: RatingModel::PerUnit {
                        rate: Decimal::from(12),
                    },
                    min_slots: None,
                    max_slots: None,
                },
            ),
        )]);
        let seats_product = ProductId::new();
        seats.product_id = Some(seats_product);
        let components = [platform, revenue, seats];
        let cells = narrowed(&components, &OptionPricing::new(only.id, "EUR"));
        let catalog = Catalog::default();
        let lead = |priced: &Priced| {
            priced
                .extras
                .iter()
                .filter(|extra| extra.lead)
                .map(|extra| extra.kind)
                .collect::<Vec<_>>()
        };

        let by_seat = in_order(
            priced(&cells, card_period(&cells), &catalog),
            &[seats_product],
            &[],
        );
        assert_eq!(lead(&by_seat), vec![CardExtraKind::PerSeat]);
        assert_eq!(read_levels(&by_seat).len(), 3, "the levels stay offered");

        let by_levels = in_order(
            priced(&cells, card_period(&cells), &catalog),
            &[product],
            &[],
        );
        assert_eq!(
            lead(&by_levels),
            vec![CardExtraKind::Base],
            "its overage never leads: the base price holding its fee does"
        );

        let hidden = in_order(
            priced(&cells, card_period(&cells), &catalog),
            &[],
            &[product],
        );
        assert_eq!(
            read_levels(&hidden),
            vec![
                (50_000, Decimal::from(29 + 199), None),
                (125_000, Decimal::from(29 + 479), None),
                (250_000, Decimal::from(29 + 899), None),
            ],
            "left off: its fees still offered, its overage reads as usage"
        );
    }
}
