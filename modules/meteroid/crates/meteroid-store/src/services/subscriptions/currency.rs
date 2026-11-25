//! The currency and plan option a subscription is sold in: which price of each charge it bills
//! (the option's cell in that currency), and that its add-ons are priced in that currency.
use crate::StoreResult;
use crate::domain::PriceComponent;
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::plan_options::{PlanOption, PlanOptionRef};
use crate::domain::price_cells::{OptionPricing, currencies, resolve_option, sold_options};
use crate::domain::prices::{Price, Pricing};
use crate::domain::subscription_components::CreateSubscriptionComponents;
use crate::errors::StoreError;
use crate::store::PgConn;
use common_domain::ids::{PlanOptionId, PlanVersionId, PriceComponentId, SubscriptionId, TenantId};
use diesel_models::plan_version_options::PlanVersionOptionRow;
use error_stack::Report;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(crate) struct SubscriptionCurrency {
    pub currency: String,
    /// The plan option bought; none where no plan charge is priced from it (add-ons, quotes).
    pub option: Option<PlanOption>,
}

fn invalid(message: String) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message))
}

/// "<charge> has no price in <currency> for <option>", as a plan change says of another currency.
fn no_price(charge: &str, currency: &str, option: Option<&PlanOption>) -> Report<StoreError> {
    match option {
        Some(option) => invalid(format!(
            "{charge} has no price in {currency} for {}",
            option.name
        )),
        None => invalid(format!("{charge} has no price in {currency}")),
    }
}

/// `option`'s cells in `currency`.
pub(crate) fn pricing(option: &PlanOption, currency: &str) -> OptionPricing {
    OptionPricing::new(option.id, currency)
}

/// Usage and one-time prices bill on no cadence of their own.
fn cadence_free(price: &Price) -> bool {
    matches!(price.pricing, Pricing::Usage(_) | Pricing::OneTime { .. })
}

/// The options of each of `versions`, by position.
pub(crate) async fn plan_options_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    versions: &[PlanVersionId],
) -> StoreResult<HashMap<PlanVersionId, Vec<PlanOption>>> {
    let mut by_version: HashMap<PlanVersionId, Vec<PlanOption>> = HashMap::new();
    for row in PlanVersionOptionRow::list_by_plan_version_ids(conn, tenant_id, versions)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        by_version
            .entry(row.plan_version_id)
            .or_default()
            .push(row.into());
    }
    for options in by_version.values_mut() {
        options.sort_by_key(|option| option.position);
    }
    Ok(by_version)
}

/// The alias of the option `asked` names in `version`, as a checkout keeps it.
pub(crate) async fn option_alias(
    conn: &mut PgConn,
    tenant_id: TenantId,
    version: PlanVersionId,
    asked: &PlanOptionRef,
) -> StoreResult<String> {
    let options = plan_options_of(conn, tenant_id, &[version])
        .await?
        .remove(&version)
        .unwrap_or_default();
    Ok(resolve_option(&options, Some(asked))?.alias.clone())
}

/// Records the option `subscription_id` now bills, as a plan change applies it.
pub(crate) async fn record_plan_option(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    option: Option<PlanOptionId>,
) -> StoreResult<()> {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::subscription::dsl as s;
    diesel_async::RunQueryDsl::execute(
        diesel::update(s::subscription.filter(s::id.eq(subscription_id)))
            .set(s::plan_option_id.eq(option)),
        conn,
    )
    .await
    .map_err(|error| -> Report<StoreError> {
        crate::errors::StoreErrorContainer::from(error).into()
    })?;
    Ok(())
}

/// The cadence each charge is asked on, as integrators chose a term before options; those removed
/// or overridden ask nothing.
pub(crate) fn asked_cadences(
    params: Option<&CreateSubscriptionComponents>,
) -> Vec<(PriceComponentId, BillingPeriodEnum)> {
    let Some(params) = params else {
        return vec![];
    };
    params
        .parameterized_components
        .iter()
        .filter(|p| {
            !params.remove_components.contains(&p.component_id)
                && !params
                    .overridden_components
                    .iter()
                    .any(|o| o.component_id == p.component_id)
        })
        .filter_map(|p| p.parameters.billing_period.map(|bp| (p.component_id, bp)))
        .collect()
}

/// Whether `option`'s cells of the charges `cadences` names bill on those cadences in `currency`.
/// A charge billed on one cadence in every option has nothing to choose: it bills on its own.
fn bills_on(
    option: &PlanOption,
    currency: &str,
    components: &[PriceComponent],
    cadences: &[(PriceComponentId, BillingPeriodEnum)],
) -> bool {
    let sold = pricing(option, currency);
    cadences.iter().all(|(id, cadence)| {
        components
            .iter()
            .find(|c| c.id == *id && !c.prices.is_empty())
            .is_none_or(|c| {
                let mut own: Vec<BillingPeriodEnum> = c
                    .prices
                    .iter()
                    .filter(|p| !cadence_free(p) && p.currency.eq_ignore_ascii_case(currency))
                    .map(|p| p.cadence)
                    .collect();
                own.sort();
                own.dedup();
                own.len() <= 1
                    || sold
                        .prices(c)
                        .iter()
                        .any(|p| cadence_free(p) || p.cadence == *cadence)
            })
    })
}

impl SubscriptionCurrency {
    pub fn new(currency: &str) -> Self {
        SubscriptionCurrency {
            currency: currency.trim().to_ascii_uppercase(),
            option: None,
        }
    }

    /// `currency`, the plan's charges priced from `option`'s cells.
    pub fn with_option(currency: &str, option: Option<PlanOption>) -> Self {
        SubscriptionCurrency {
            option,
            ..Self::new(currency)
        }
    }

    fn of(currency: &str, option: Option<&PlanOption>) -> Self {
        Self::with_option(currency, option.cloned())
    }

    /// A new subscription buys the option `asked`, else the one billing the `cadences` asked of
    /// its charges, else the first sold; in its customer's currency when the option is sold there,
    /// else the plan's, else the first it is sold in. Refused for an option the plan lacks.
    pub fn for_new_subscription(
        customer_currency: &str,
        plan_currency: &str,
        options: &[PlanOption],
        components: &[PriceComponent],
        asked: Option<&PlanOptionRef>,
        cadences: &[(PriceComponentId, BillingPeriodEnum)],
    ) -> StoreResult<Self> {
        let named = match asked {
            Some(asked) if !options.is_empty() => Some(resolve_option(options, Some(asked))?),
            Some(asked) => return Err(invalid(format!("The plan has no option {asked}"))),
            None => None,
        };
        // A plan pricing nothing per currency (legacy or extra components only) keeps its own.
        if components.iter().all(|c| c.prices.is_empty()) || options.is_empty() {
            let option = named.or_else(|| options.first());
            return Ok(Self::of(plan_currency, option));
        }
        // A pre-currency price is in the plan's currency only.
        let legacy = components
            .iter()
            .any(|c| c.prices.is_empty() && c.legacy_pricing.is_some());
        let mut candidates = vec![customer_currency.to_owned(), plan_currency.to_owned()];
        candidates.extend(currencies(components));
        let candidates: Vec<String> = candidates
            .into_iter()
            .map(|c| c.trim().to_ascii_uppercase())
            .filter(|c| !legacy || c.eq_ignore_ascii_case(plan_currency))
            .collect();

        if let Some(option) = named {
            let currency = candidates
                .iter()
                .find(|c| pricing(option, c).sells(components))
                .map_or(plan_currency, String::as_str);
            return Ok(Self::of(currency, Some(option)));
        }
        for currency in &candidates {
            let sold = sold_options(options, components, currency);
            if let Some(option) = sold
                .into_iter()
                .find(|o| bills_on(o, currency, components, cadences))
            {
                return Ok(Self::of(currency, Some(option)));
            }
        }
        // The cadences asked are no option's: name the charge none bills on its cadence.
        if let Some((id, cadence)) = cadences.iter().find(|(id, cadence)| {
            !options.iter().any(|o| {
                candidates
                    .iter()
                    .any(|c| bills_on(o, c, components, &[(*id, *cadence)]))
            })
        }) && let Some(charge) = components.iter().find(|c| c.id == *id)
        {
            return Err(invalid(format!(
                "{} is not priced on {cadence:?} in any option of the plan",
                charge.name
            )));
        }
        if !cadences.is_empty() {
            return Err(invalid(
                "No option of the plan bills its charges on the periods asked: name the option"
                    .to_owned(),
            ));
        }
        // Sold nowhere: the plan's currency, where the charge lacking a price is named.
        Ok(Self::of(plan_currency, options.first()))
    }

    /// The prices `component` sells at: the option's cell in this currency, else every price of
    /// the currency (no option known). Refused when there is none; a charge with no price at all
    /// (included) has none to sell.
    pub fn cell(&self, component: &PriceComponent) -> StoreResult<Vec<Price>> {
        if component.prices.is_empty() {
            return Ok(Vec::new());
        }
        let prices: Vec<Price> = match &self.option {
            Some(option) => pricing(option, &self.currency)
                .prices(component)
                .into_iter()
                .cloned()
                .collect(),
            None => component
                .prices
                .iter()
                .filter(|p| p.currency.eq_ignore_ascii_case(&self.currency))
                .cloned()
                .collect(),
        };
        if prices.is_empty() {
            return Err(no_price(
                &component.name,
                &self.currency,
                self.option.as_ref(),
            ));
        }
        Ok(prices)
    }

    /// An add-on is sold with the subscription only at a price in its currency; it bills on its
    /// own cadence.
    pub fn accept_add_on(&self, add_on: &str, price: &Price) -> StoreResult<()> {
        if price.currency.eq_ignore_ascii_case(&self.currency) {
            return Ok(());
        }
        Err(no_price(add_on, &self.currency, None))
    }

    /// `components` priced as a new subscription in this currency and option is: each one's
    /// cell; those `skip` names (removed, overridden) keep theirs.
    pub fn price_components(
        &self,
        components: &mut [PriceComponent],
        skip: impl Fn(&PriceComponentId) -> bool,
    ) -> StoreResult<()> {
        for component in components {
            if skip(&component.id) {
                continue;
            }
            component.prices = self.cell(component)?;
        }
        Ok(())
    }
}

/// The term an add-on joins a new subscription on: the one it is sold on, else the shortest
/// recurring period of its charges.
pub(crate) fn add_on_term(
    term: Option<BillingPeriodEnum>,
    components: &[crate::domain::SubscriptionComponentNewInternal],
) -> Option<BillingPeriodEnum> {
    term.or_else(|| {
        components
            .iter()
            .filter_map(|c| c.period.as_billing_period_opt())
            .min()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::price_components::OptionPrice;
    use common_domain::ids::{BaseId, PriceId, ProductId};
    use rust_decimal::Decimal;

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

    fn price(currency: &str, cadence: BillingPeriodEnum, rate: i64) -> Price {
        Price {
            id: PriceId::new(),
            product_id: ProductId::default(),
            cadence,
            currency: currency.to_owned(),
            pricing: Pricing::Rate {
                rate: Decimal::from(rate),
            },
            tenant_id: TenantId::default(),
            created_at: chrono::NaiveDateTime::default(),
            archived_at: None,
            catalog: true,
            credit_unit: None,
        }
    }

    /// A charge priced `cells` (option, price), each price linked in its option.
    fn charge(name: &str, cells: Vec<(&PlanOption, Price)>) -> PriceComponent {
        PriceComponent {
            id: PriceComponentId::new(),
            name: name.to_owned(),
            fee_structure: None,
            product_id: Some(ProductId::default()),
            option_prices: cells
                .iter()
                .map(|(o, p)| OptionPrice {
                    option_id: o.id,
                    price_id: p.id,
                })
                .collect(),
            prices: cells.into_iter().map(|(_, p)| p).collect(),
            legacy_pricing: None,
        }
    }

    #[test]
    fn an_option_is_asked_by_alias_and_sold_in_the_plan_currency_as_a_fallback() {
        let monthly = option("Monthly", 0);
        let annual = option("Annual", 1);
        let platform = charge(
            "Platform",
            vec![
                (&monthly, price("USD", BillingPeriodEnum::Monthly, 10)),
                (&annual, price("USD", BillingPeriodEnum::Annual, 100)),
                (&monthly, price("EUR", BillingPeriodEnum::Monthly, 9)),
            ],
        );
        let options = vec![monthly.clone(), annual.clone()];
        let components = vec![platform];
        let sold = SubscriptionCurrency::for_new_subscription(
            "EUR",
            "USD",
            &options,
            &components,
            Some(&PlanOptionRef::Alias("annual".to_owned())),
            &[],
        )
        .unwrap();
        assert_eq!(sold.currency, "USD", "Annual is not sold in EUR");
        assert_eq!(sold.option.as_ref().map(|o| o.id), Some(annual.id));
        assert_eq!(
            sold.cell(&components[0]).unwrap()[0].id,
            components[0].prices[1].id
        );

        let default = SubscriptionCurrency::for_new_subscription(
            "EUR",
            "USD",
            &options,
            &components,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(default.currency, "EUR");
        assert_eq!(default.option.map(|o| o.id), Some(monthly.id));

        assert!(
            SubscriptionCurrency::for_new_subscription(
                "EUR",
                "USD",
                &options,
                &components,
                Some(&PlanOptionRef::Alias("weekly".to_owned())),
                &[],
            )
            .is_err()
        );
    }

    #[test]
    fn a_cadence_asked_picks_the_option_billing_on_it() {
        let monthly = option("Monthly", 0);
        let annual = option("Annual", 1);
        let platform = charge(
            "Platform",
            vec![
                (&monthly, price("USD", BillingPeriodEnum::Monthly, 10)),
                (&annual, price("USD", BillingPeriodEnum::Annual, 100)),
            ],
        );
        let options = vec![monthly, annual.clone()];
        let id = platform.id;
        let components = vec![platform];
        let sold = SubscriptionCurrency::for_new_subscription(
            "USD",
            "USD",
            &options,
            &components,
            None,
            &[(id, BillingPeriodEnum::Annual)],
        )
        .unwrap();
        assert_eq!(sold.option.map(|o| o.id), Some(annual.id));
        let refused = SubscriptionCurrency::for_new_subscription(
            "USD",
            "USD",
            &options,
            &components,
            None,
            &[(id, BillingPeriodEnum::Quarterly)],
        );
        assert!(
            format!("{:?}", refused.unwrap_err()).contains("Platform"),
            "the charge is named"
        );
    }
}
