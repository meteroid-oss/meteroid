//! Price cells: a charge's prices in one plan option and one currency, several only for capacity
//! levels. A price's cadence is how it bills, never what a buyer picks: the buyer picks an option
//! (`plan_options`). An option is sold in a currency when every priced charge has a cell there.
use super::enums::BillingPeriodEnum;
use super::plan_options::{PlanOption, PlanOptionRef};
use super::price_components::PriceComponent;
use super::prices::{Price, Pricing};
use crate::StoreResult;
use crate::errors::StoreError;
use common_domain::ids::PlanOptionId;
use error_stack::Report;

/// Usage and one-time prices have no cadence of their own.
pub fn cadence_free(pricing: &Pricing) -> bool {
    matches!(pricing, Pricing::Usage(_) | Pricing::OneTime { .. })
}

fn same_currency(price: &Price, currency: &str) -> bool {
    price.currency.eq_ignore_ascii_case(currency.trim())
}

/// The prices of `prices` in `currency` on `cadence` (a usage or one-time price is on any); no
/// cadence asked matches any. Add-ons are priced per currency and cadence, without options.
pub fn cell<'a>(
    prices: &'a [Price],
    currency: &str,
    cadence: Option<BillingPeriodEnum>,
) -> Vec<&'a Price> {
    prices
        .iter()
        .filter(|price| same_currency(price, currency))
        .filter(|price| {
            cadence.is_none_or(|cadence| cadence_free(&price.pricing) || price.cadence == cadence)
        })
        .collect()
}

/// The one cadence an add-on's fixed prices share across every currency, when `currency` prices
/// it: the add-on bills on it whatever the subscription's period.
pub fn single_cadence(prices: &[Price], currency: &str) -> Option<BillingPeriodEnum> {
    let mut cadences: Vec<BillingPeriodEnum> = prices
        .iter()
        .filter(|p| !cadence_free(&p.pricing))
        .map(|p| p.cadence)
        .collect();
    cadences.sort();
    cadences.dedup();
    let priced_here = prices
        .iter()
        .any(|p| !cadence_free(&p.pricing) && same_currency(p, currency));
    match cadences.as_slice() {
        [only] if priced_here => Some(*only),
        _ => None,
    }
}

/// The prices of `prices` in `currency`, whatever their cadence. Empty: not priced in it.
pub fn currency_prices(prices: &[Price], currency: &str) -> Vec<Price> {
    cell(prices, currency, None).into_iter().cloned().collect()
}

/// A charge with a price in some cell; one without (an included product) needs none.
fn priced(component: &PriceComponent) -> bool {
    !component.prices.is_empty()
}

/// The cell of `component` for `(option, currency)`: the prices it links there. An archived
/// price a version still links stays billed.
pub fn option_cell<'a>(
    component: &'a PriceComponent,
    option: PlanOptionId,
    currency: &str,
) -> Vec<&'a Price> {
    component
        .prices
        .iter()
        .filter(|price| same_currency(price, currency))
        .filter(|price| {
            component
                .option_prices
                .iter()
                .any(|link| link.option_id == option && link.price_id == price.id)
        })
        .collect()
}

/// The cells of one option in one currency, as a customer billed in it buys a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionPricing {
    pub option: PlanOptionId,
    pub currency: String,
}

impl OptionPricing {
    pub fn new(option: PlanOptionId, currency: &str) -> Self {
        OptionPricing {
            option,
            currency: currency.trim().to_ascii_uppercase(),
        }
    }

    pub fn prices<'a>(&self, component: &'a PriceComponent) -> Vec<&'a Price> {
        option_cell(component, self.option, &self.currency)
    }

    /// Whether every priced charge has a cell here; a plan of included products only is sold
    /// anywhere.
    pub fn sells(&self, components: &[PriceComponent]) -> bool {
        components
            .iter()
            .filter(|c| priced(c))
            .all(|c| !self.prices(c).is_empty())
    }

    /// Keeps each charge's cell only, empty when it has none here. Components without prices of
    /// their own (legacy, included only) stay.
    pub fn narrow(&self, components: &mut [PriceComponent]) {
        for component in components {
            if !priced(component) {
                continue;
            }
            let kept: Vec<Price> = self.prices(component).into_iter().cloned().collect();
            component.option_prices.retain(|link| {
                link.option_id == self.option && kept.iter().any(|p| p.id == link.price_id)
            });
            component.prices = kept;
        }
    }
}

/// The options of `options` sold in `currency`, in their order.
pub fn sold_options<'a>(
    options: &'a [PlanOption],
    components: &[PriceComponent],
    currency: &str,
) -> Vec<&'a PlanOption> {
    let mut sold: Vec<&PlanOption> = options
        .iter()
        .filter(|option| OptionPricing::new(option.id, currency).sells(components))
        .collect();
    // Shown options first: a default picked as "the first sold" never lands on a hidden one.
    sold.sort_by_key(|option| option.hidden);
    sold
}

/// The option asked, or the default (the first) when none is.
pub fn resolve_option<'a>(
    options: &'a [PlanOption],
    asked: Option<&PlanOptionRef>,
) -> StoreResult<&'a PlanOption> {
    let found = match asked {
        None => options.iter().min_by_key(|option| option.position),
        Some(asked) => options.iter().find(|option| asked.matches(option)),
    };
    found.ok_or_else(|| {
        Report::new(StoreError::InvalidArgument(match asked {
            None => "This plan has no option".to_owned(),
            Some(asked) => format!("This plan has no option {asked}"),
        }))
    })
}

/// Every currency `components` are priced in, sorted.
pub fn currencies(components: &[PriceComponent]) -> Vec<String> {
    let mut currencies: Vec<String> = components
        .iter()
        .flat_map(|c| c.prices.iter())
        .map(|p| p.currency.trim().to_ascii_uppercase())
        .collect();
    currencies.sort();
    currencies.dedup();
    currencies
}

/// Every `(currency, option)` sold, by currency then option order.
pub fn priced_options(
    options: &[PlanOption],
    components: &[PriceComponent],
) -> Vec<(String, PlanOptionId)> {
    currencies(components)
        .into_iter()
        .flat_map(|currency| {
            sold_options(options, components, &currency)
                .into_iter()
                .map(|option| (currency.clone(), option.id))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::plan_options::alias_from_name;
    use crate::domain::price_components::OptionPrice;
    use common_domain::ids::{BaseId, PlanVersionId, PriceId, ProductId, TenantId};
    use rust_decimal::Decimal;

    fn price(currency: &str, cadence: BillingPeriodEnum, pricing: Pricing) -> Price {
        Price {
            id: PriceId::new(),
            product_id: ProductId::default(),
            cadence,
            currency: currency.to_owned(),
            pricing,
            tenant_id: TenantId::default(),
            created_at: chrono::NaiveDateTime::default(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    fn rate(amount: i64) -> Pricing {
        Pricing::Rate {
            rate: Decimal::from(amount),
        }
    }

    fn capacity(included: u64) -> Pricing {
        Pricing::Capacity {
            rate: Decimal::from(10),
            included,
            overage_rate: Decimal::ONE,
        }
    }

    fn usage() -> Pricing {
        Pricing::Usage(super::super::price_components::UsagePricingModel::PerUnit {
            rate: Decimal::ONE,
        })
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

    /// A charge whose each price is linked in the options listed with it.
    fn component(cells: Vec<(Vec<PlanOptionId>, Price)>) -> PriceComponent {
        PriceComponent {
            id: common_domain::ids::PriceComponentId::new(),
            name: "charge".to_owned(),
            fee_structure: None,
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

    fn included() -> PriceComponent {
        component(vec![])
    }

    #[test]
    fn a_cell_is_one_option_in_one_currency() {
        let (monthly, annual) = (option("Monthly", 0), option("Annual", 1));
        let both = vec![monthly.id, annual.id];
        let platform = component(vec![
            (
                vec![monthly.id],
                price("USD", BillingPeriodEnum::Monthly, rate(10)),
            ),
            (
                vec![annual.id],
                price("USD", BillingPeriodEnum::Annual, rate(100)),
            ),
            (both, price("EUR", BillingPeriodEnum::Monthly, rate(9))),
        ]);
        let ids = |cell: Vec<&Price>| cell.iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(
            ids(option_cell(&platform, annual.id, "usd")),
            vec![platform.prices[1].id]
        );
        assert_eq!(
            ids(option_cell(&platform, monthly.id, "USD")),
            vec![platform.prices[0].id]
        );
        // The same price in every option, its cadence only how it bills.
        assert_eq!(
            ids(option_cell(&platform, annual.id, "EUR")),
            vec![platform.prices[2].id]
        );
        assert!(option_cell(&platform, annual.id, "GBP").is_empty());
        assert!(option_cell(&platform, PlanOptionId::new(), "USD").is_empty());
    }

    #[test]
    fn capacity_levels_are_all_of_the_cell() {
        let standard = option("Standard", 0);
        let commit = component(vec![
            (
                vec![standard.id],
                price("USD", BillingPeriodEnum::Monthly, capacity(1_000)),
            ),
            (
                vec![standard.id],
                price("USD", BillingPeriodEnum::Monthly, capacity(10_000)),
            ),
        ]);
        assert_eq!(option_cell(&commit, standard.id, "USD").len(), 2);
    }

    #[test]
    fn an_option_is_sold_where_every_priced_charge_has_a_cell() {
        let (monthly, annual) = (option("Monthly", 0), option("Annual", 1));
        let both = vec![monthly.id, annual.id];
        let platform = component(vec![
            (
                vec![monthly.id],
                price("USD", BillingPeriodEnum::Monthly, rate(10)),
            ),
            (
                vec![annual.id],
                price("USD", BillingPeriodEnum::Annual, rate(100)),
            ),
            (
                vec![monthly.id],
                price("EUR", BillingPeriodEnum::Monthly, rate(9)),
            ),
        ]);
        let calls = component(vec![
            (
                both.clone(),
                price("USD", BillingPeriodEnum::Monthly, usage()),
            ),
            (both, price("EUR", BillingPeriodEnum::Monthly, usage())),
        ]);
        let components = vec![platform, calls, included()];

        assert!(OptionPricing::new(monthly.id, "usd").sells(&components));
        assert!(OptionPricing::new(annual.id, "USD").sells(&components));
        assert!(OptionPricing::new(monthly.id, "EUR").sells(&components));
        assert!(
            !OptionPricing::new(annual.id, "EUR").sells(&components),
            "the platform fee has no EUR price in Annual"
        );
        assert!(!OptionPricing::new(monthly.id, "GBP").sells(&components));
        assert!(
            OptionPricing::new(annual.id, "GBP").sells(&[included()]),
            "included products need no price"
        );

        let options = vec![monthly.clone(), annual.clone()];
        let sold = |currency: &str| {
            sold_options(&options, &components, currency)
                .into_iter()
                .map(|o| o.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(sold("USD"), vec![monthly.id, annual.id]);
        assert_eq!(sold("EUR"), vec![monthly.id]);
        assert!(sold("GBP").is_empty());

        assert_eq!(currencies(&components), vec!["EUR", "USD"]);
        assert_eq!(
            priced_options(&options, &components),
            vec![
                ("EUR".to_owned(), monthly.id),
                ("USD".to_owned(), monthly.id),
                ("USD".to_owned(), annual.id),
            ]
        );
    }

    #[test]
    fn narrowing_keeps_the_cell_of_the_option() {
        let (monthly, annual) = (option("Monthly", 0), option("Annual", 1));
        let both = vec![monthly.id, annual.id];
        let platform = component(vec![
            (
                vec![monthly.id],
                price("USD", BillingPeriodEnum::Monthly, rate(10)),
            ),
            (
                vec![annual.id],
                price("USD", BillingPeriodEnum::Annual, rate(100)),
            ),
        ]);
        let support = component(vec![(
            both,
            price("USD", BillingPeriodEnum::Monthly, rate(3)),
        )]);
        let components = vec![platform, support, included()];

        let mut yearly = components.clone();
        OptionPricing::new(annual.id, "USD").narrow(&mut yearly);
        assert_eq!(yearly[0].prices.len(), 1);
        assert_eq!(yearly[0].prices[0].id, components[0].prices[1].id);
        assert_eq!(
            yearly[1].prices[0].id, components[1].prices[0].id,
            "a monthly fee linked in Annual bills monthly there"
        );
        assert!(
            yearly[1]
                .option_prices
                .iter()
                .all(|link| link.option_id == annual.id)
        );
        assert!(yearly[2].prices.is_empty());

        let mut euros = components.clone();
        OptionPricing::new(annual.id, "EUR").narrow(&mut euros);
        assert!(euros[0].prices.is_empty(), "not priced in EUR");
    }

    #[test]
    fn the_option_asked_or_the_first() {
        let options = vec![option("Monthly", 0), option("Annual", 1)];
        assert_eq!(resolve_option(&options, None).unwrap().id, options[0].id);
        assert_eq!(
            resolve_option(&options, Some(&PlanOptionRef::Alias("Annual".to_owned())))
                .unwrap()
                .id,
            options[1].id
        );
        assert_eq!(
            resolve_option(&options, Some(&PlanOptionRef::Id(options[1].id)))
                .unwrap()
                .id,
            options[1].id
        );
        let unknown = resolve_option(&options, Some(&PlanOptionRef::Alias("startup".to_owned())))
            .unwrap_err();
        assert!(
            matches!(unknown.current_context(), StoreError::InvalidArgument(why) if why.contains("startup"))
        );
        assert!(resolve_option(&[], None).is_err());
    }

    #[test]
    fn an_alias_is_the_slug_of_the_name() {
        assert_eq!(alias_from_name("Annual", &[]), "annual");
        assert_eq!(alias_from_name("Annual 2", &[]), "annual-2");
        assert_eq!(alias_from_name("  Pro / Startup!  ", &[]), "pro-startup");
        assert_eq!(alias_from_name("Été", &[]), "t");
        assert_eq!(alias_from_name("—", &[]), "option");
        assert_eq!(
            alias_from_name("Annual", &["annual".to_owned()]),
            "annual-2"
        );
        assert_eq!(
            alias_from_name("Annual", &["annual".to_owned(), "annual-2".to_owned()]),
            "annual-3"
        );
        let long = alias_from_name(&"x".repeat(60), &["x".repeat(40)]);
        assert_eq!(long.len(), 40);
        assert!(long.ends_with("-2"));
        assert!(crate::domain::plan_options::valid_alias(&long));
    }

    #[test]
    fn an_add_on_bills_on_its_single_cadence() {
        let prices = vec![
            price("USD", BillingPeriodEnum::Annual, rate(100)),
            price("EUR", BillingPeriodEnum::Annual, rate(90)),
            price("USD", BillingPeriodEnum::Monthly, usage()),
        ];
        assert_eq!(
            single_cadence(&prices, "USD"),
            Some(BillingPeriodEnum::Annual)
        );
        assert_eq!(
            cell(&prices, "USD", Some(BillingPeriodEnum::Quarterly)).len(),
            1,
            "usage is on any cadence"
        );
        assert_eq!(currency_prices(&prices, "eur").len(), 1);
    }
}
