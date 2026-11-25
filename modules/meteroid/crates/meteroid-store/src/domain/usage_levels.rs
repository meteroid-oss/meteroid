//! The level a usage-levels charge (R-135) starts on, or lands on at a plan change: one of the
//! capacity prices of its plan component's cell, named by the units it includes. Components are
//! read narrowed to the cell sold (`OptionPricing::narrow`).
use super::price_components::PriceComponent;
use super::prices::{Price, Pricing};
use super::subscription_components::{
    ComponentParameterization, ComponentParameters, CreateSubscriptionComponents,
    SubscriptionComponent, SubscriptionFee,
};
use crate::errors::StoreError;
use common_domain::ids::{PriceComponentId, ProductId};

/// A level picked for a usage-levels charge, by the units it includes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChosenLevel {
    /// The charge's product; none: the plan's only usage-levels charge.
    pub product_id: Option<ProductId>,
    pub included: u64,
}

fn included_of(price: &Price) -> Option<u64> {
    match price.pricing {
        Pricing::Capacity { included, .. } => Some(included),
        _ => None,
    }
}

/// The levels `component` is sold at, fewest units first: the capacity prices of its cell when it
/// has several. Empty for a charge sold at one level, or not a capacity.
pub fn levels(component: &PriceComponent) -> Vec<&Price> {
    if !component.prices.iter().all(|p| included_of(p).is_some()) {
        return Vec::new();
    }
    let mut levels: Vec<&Price> = component.prices.iter().collect();
    levels.sort_by_key(|p: &&Price| included_of(p));
    levels.dedup_by_key(|p: &mut &Price| included_of(p));
    if levels.len() < 2 {
        levels.clear();
    }
    levels
}

/// The units each of `levels` includes, fewest first.
pub fn sizes(levels: &[&Price]) -> Vec<u64> {
    levels.iter().filter_map(|p| included_of(p)).collect()
}

fn parameterization(component_id: PriceComponentId, included: u64) -> ComponentParameterization {
    ComponentParameterization {
        component_id,
        parameters: ComponentParameters {
            initial_slot_count: None,
            billing_period: None,
            committed_capacity: Some(included),
        },
    }
}

/// `included` when it is a level of `component`, else refused naming its levels.
fn check(component: &PriceComponent, levels: &[&Price], included: u64) -> Result<u64, StoreError> {
    let sizes = sizes(levels);
    if sizes.contains(&included) {
        return Ok(included);
    }
    let sizes: Vec<String> = sizes.iter().map(u64::to_string).collect();
    Err(StoreError::InvalidArgument(format!(
        "{included} is not a level of {}: its levels include {}",
        component.name,
        sizes.join(", ")
    )))
}

/// The level each usage-levels charge of `components` starts on: the one `chosen`, else its
/// lowest, the level a pricing page reads the plan's price from ("from"). Refused: a level not of
/// the charge's cell, a product without such a charge, a level naming no product where the plan
/// has several such charges.
pub fn starting_levels(
    components: &[PriceComponent],
    chosen: &[ChosenLevel],
) -> Result<Vec<ComponentParameterization>, StoreError> {
    let charges: Vec<(&PriceComponent, Vec<&Price>)> = components
        .iter()
        .map(|c| (c, levels(c)))
        .filter(|(_, levels)| !levels.is_empty())
        .collect();
    fn names(pick: &ChosenLevel, component: &PriceComponent) -> bool {
        pick.product_id
            .is_none_or(|product| component.product_id == Some(product))
    }
    for pick in chosen {
        match charges.iter().filter(|(c, _)| names(pick, c)).count() {
            0 => {
                return Err(StoreError::InvalidArgument(match pick.product_id {
                    Some(product) => format!("This plan has no usage levels for product {product}"),
                    None => "This plan has no usage levels to choose from".to_owned(),
                }));
            }
            1 => {}
            _ if pick.product_id.is_none() => {
                return Err(StoreError::InvalidArgument(
                    "This plan has several charges with usage levels: name the product of each level"
                        .to_owned(),
                ));
            }
            _ => {}
        }
    }
    charges
        .iter()
        .map(|(component, levels)| {
            let included = match chosen.iter().find(|pick| names(pick, component)) {
                Some(pick) => check(component, levels, pick.included)?,
                None => sizes(levels)[0],
            };
            Ok(parameterization(component.id, included))
        })
        .collect()
}

/// `asked` with the level of every usage-levels charge of `components` it neither overrides nor
/// removes: the one it asks, checked against the cell, else the lowest (`starting_levels`).
pub fn settle_levels(
    components: &[PriceComponent],
    asked: &mut CreateSubscriptionComponents,
) -> Result<(), StoreError> {
    for component in components {
        let levels = levels(component);
        if levels.is_empty()
            || asked.remove_components.contains(&component.id)
            || asked
                .overridden_components
                .iter()
                .any(|o| o.component_id == component.id)
        {
            continue;
        }
        let lowest = sizes(&levels)[0];
        match asked
            .parameterized_components
            .iter_mut()
            .find(|p| p.component_id == component.id)
        {
            Some(param) => match param.parameters.committed_capacity {
                Some(included) => {
                    check(component, &levels, included)?;
                }
                None => param.parameters.committed_capacity = Some(lowest),
            },
            None => asked
                .parameterized_components
                .push(parameterization(component.id, lowest)),
        }
    }
    Ok(())
}

/// A usage-levels charge of the plan a change moves to, and the level it lands on.
#[derive(Clone, Debug)]
pub struct LevelPick<'a> {
    pub component: &'a PriceComponent,
    /// Fewest units first.
    pub levels: Vec<&'a Price>,
    pub included: u64,
}

impl LevelPick<'_> {
    pub fn parameterization(&self) -> ComponentParameterization {
        parameterization(self.component.id, self.included)
    }
}

/// The level a plan change lands each usage-levels charge of `target` on: the one `chosen` (by
/// the target's component), else the level held of its product when the target sells it, else the
/// lowest above it, else the highest; the lowest for a product not held. The backoffice's plan
/// change picks alike (`componentChoices.configureMatched`).
pub fn plan_change_levels<'a>(
    current: &[SubscriptionComponent],
    target: &'a [PriceComponent],
    chosen: &[(PriceComponentId, u64)],
) -> Result<Vec<LevelPick<'a>>, StoreError> {
    let picks: Vec<(&PriceComponent, Vec<&Price>)> = target
        .iter()
        .map(|c| (c, levels(c)))
        .filter(|(_, levels)| !levels.is_empty())
        .collect();
    if let Some((id, _)) = chosen
        .iter()
        .find(|(id, _)| !picks.iter().any(|(c, _)| c.id == *id))
    {
        return Err(StoreError::InvalidArgument(format!(
            "Component {id} has no usage levels to choose from"
        )));
    }
    picks
        .into_iter()
        .map(|(component, levels)| {
            let included = match chosen.iter().find(|(id, _)| *id == component.id) {
                Some((_, included)) => check(component, &levels, *included)?,
                None => {
                    let sizes = sizes(&levels);
                    let held = component.product_id.and_then(|product| {
                        current
                            .iter()
                            .filter(|c| c.product_id == Some(product))
                            .find_map(|c| match c.fee {
                                SubscriptionFee::Capacity { included, .. } => Some(included),
                                _ => None,
                            })
                    });
                    match held {
                        Some(held) => sizes
                            .iter()
                            .copied()
                            .find(|size| *size >= held)
                            .unwrap_or(sizes[sizes.len() - 1]),
                        None => sizes[0],
                    }
                }
            };
            Ok(LevelPick {
                component,
                levels,
                included,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
    use common_domain::ids::{
        BaseId, PriceId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
    };
    use rust_decimal::Decimal;

    fn level(product: ProductId, included: u64) -> Price {
        Price {
            id: PriceId::new(),
            product_id: product,
            cadence: BillingPeriodEnum::Monthly,
            currency: "EUR".to_owned(),
            pricing: Pricing::Capacity {
                rate: Decimal::from(included / 100),
                included,
                overage_rate: Decimal::new(5, 3),
            },
            tenant_id: TenantId::default(),
            created_at: chrono::NaiveDateTime::default(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    fn charge(product: ProductId, sizes: &[u64]) -> PriceComponent {
        PriceComponent {
            id: PriceComponentId::new(),
            name: "Managed revenue".to_owned(),
            product_id: Some(product),
            prices: sizes.iter().map(|s| level(product, *s)).collect(),
            option_prices: vec![],
            legacy_pricing: None,
        }
    }

    fn held(product: ProductId, included: u64) -> SubscriptionComponent {
        SubscriptionComponent {
            id: SubscriptionPriceComponentId::new(),
            price_component_id: None,
            product_id: Some(product),
            subscription_id: SubscriptionId::new(),
            name: "Managed revenue".to_owned(),
            period: SubscriptionFeeBillingPeriod::Monthly,
            fee: SubscriptionFee::Capacity {
                metric_id: Default::default(),
                rate: Decimal::from(479),
                included,
                overage_rate: Decimal::new(5, 3),
                credit_unit: None,
            },
            price_id: None,
            effective_from: chrono::NaiveDate::default(),
            effective_to: None,
            lineage_id: None,
            added_by_amendment: false,
        }
    }

    fn committed(params: &[ComponentParameterization]) -> Vec<Option<u64>> {
        params
            .iter()
            .map(|p| p.parameters.committed_capacity)
            .collect()
    }

    #[test]
    fn a_charge_at_one_level_has_none_to_choose() {
        let product = ProductId::new();
        assert!(levels(&charge(product, &[50_000])).is_empty());
        assert_eq!(
            sizes(&levels(&charge(product, &[250_000, 50_000, 125_000]))),
            vec![50_000, 125_000, 250_000]
        );
    }

    #[test]
    fn a_new_subscription_starts_on_the_level_chosen_else_the_lowest() {
        let product = ProductId::new();
        let plan = vec![charge(product, &[50_000, 125_000, 250_000])];
        assert_eq!(
            committed(&starting_levels(&plan, &[]).unwrap()),
            [Some(50_000)]
        );
        let chosen = ChosenLevel {
            product_id: None,
            included: 125_000,
        };
        assert_eq!(
            committed(&starting_levels(&plan, &[chosen]).unwrap()),
            [Some(125_000)]
        );
        let named = ChosenLevel {
            product_id: Some(product),
            included: 250_000,
        };
        assert_eq!(
            committed(&starting_levels(&plan, &[named]).unwrap()),
            [Some(250_000)]
        );
    }

    #[test]
    fn a_level_not_of_the_cell_or_without_its_charge_is_refused() {
        let product = ProductId::new();
        let plan = vec![charge(product, &[50_000, 125_000])];
        let off_cell = ChosenLevel {
            product_id: None,
            included: 1_000_000,
        };
        assert!(starting_levels(&plan, &[off_cell]).is_err());
        let other_product = ChosenLevel {
            product_id: Some(ProductId::new()),
            included: 50_000,
        };
        assert!(starting_levels(&plan, &[other_product]).is_err());
        let two = vec![
            charge(product, &[50_000, 125_000]),
            charge(ProductId::new(), &[10, 20]),
        ];
        let unnamed = ChosenLevel {
            product_id: None,
            included: 125_000,
        };
        assert!(starting_levels(&two, &[unnamed]).is_err());
    }

    #[test]
    fn a_checkout_gets_the_lowest_level_unless_it_asks_one_of_the_cell() {
        let product = ProductId::new();
        let plan = vec![charge(product, &[50_000, 125_000])];
        let mut asked = CreateSubscriptionComponents {
            parameterized_components: vec![],
            overridden_components: vec![],
            extra_components: vec![],
            remove_components: vec![],
        };
        settle_levels(&plan, &mut asked).unwrap();
        assert_eq!(committed(&asked.parameterized_components), [Some(50_000)]);
        asked.parameterized_components[0]
            .parameters
            .committed_capacity = Some(125_000);
        settle_levels(&plan, &mut asked).unwrap();
        assert_eq!(committed(&asked.parameterized_components), [Some(125_000)]);
        asked.parameterized_components[0]
            .parameters
            .committed_capacity = Some(60_000);
        assert!(settle_levels(&plan, &mut asked).is_err());
    }

    #[test]
    fn a_plan_change_keeps_the_level_held_else_the_next_up_else_the_highest() {
        let product = ProductId::new();
        let target = vec![charge(product, &[50_000, 250_000, 1_000_000])];
        let picked = |held_units: u64| {
            plan_change_levels(&[held(product, held_units)], &target, &[]).unwrap()[0].included
        };
        assert_eq!(picked(250_000), 250_000);
        assert_eq!(picked(125_000), 250_000);
        assert_eq!(picked(2_000_000), 1_000_000);
        // A product not held starts on the lowest.
        assert_eq!(
            plan_change_levels(&[], &target, &[]).unwrap()[0].included,
            50_000
        );
        // The customer's own pick, of the target's levels only.
        let id = target[0].id;
        assert_eq!(
            plan_change_levels(&[held(product, 125_000)], &target, &[(id, 50_000)]).unwrap()[0]
                .included,
            50_000
        );
        assert!(plan_change_levels(&[], &target, &[(id, 125_000)]).is_err());
        assert!(plan_change_levels(&[], &target, &[(PriceComponentId::new(), 50_000)]).is_err());
    }
}
