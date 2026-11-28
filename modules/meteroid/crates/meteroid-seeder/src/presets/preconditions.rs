//! What a scenario must satisfy before its tenant is created, checked synchronously so a sandbox
//! never starts seeding something it cannot finish.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, CustomerId};
use meteroid_store::StoreResult;
use meteroid_store::domain::eligibility::{CustomerFacts, check};
use meteroid_store::errors::StoreError;

use crate::presets::scenarios::domain::{Customer, FeeType, HighlightSpec, Scenario, UnitPricing};
use crate::presets::scenarios::timeline::{Action, Clock, resolve_events};
use crate::presets::{account_credit, credit_traffic, credits};

fn ensure(holds: bool, message: impl FnOnce() -> String) -> StoreResult<()> {
    if holds {
        Ok(())
    } else {
        Err(StoreError::InvalidArgument(message()).into())
    }
}

pub(super) fn check_scenario(scenario: &Scenario, now: NaiveDateTime) -> StoreResult<()> {
    let metrics: HashSet<&str> = scenario.metrics.iter().map(|m| m.code.as_str()).collect();
    let units: HashSet<&str> = scenario.credit_units.iter().map(|u| u.code).collect();
    let features: HashSet<&str> = scenario.features.iter().map(|f| f.code.as_str()).collect();

    for feature in &scenario.features {
        if let Some(code) = feature.metric_code() {
            ensure(metrics.contains(code), || {
                format!("feature {} meters an unknown metric {code}", feature.code)
            })?;
        }
        if let Some(product) = feature.product {
            ensure(
                scenario
                    .plans
                    .iter()
                    .any(|p| p.components.iter().any(|c| c.name == product)),
                || {
                    format!(
                        "feature {} is on {product}, which no plan sells",
                        feature.code
                    )
                },
            )?;
        }
    }
    for card in &scenario.rate_cards {
        ensure(metrics.contains(card.metric_code), || {
            format!(
                "rate card {} rates an unknown metric {}",
                card.code, card.metric_code
            )
        })?;
        ensure(
            card.rates
                .iter()
                .all(|row| row.key.len() == card.dimensions.len()),
            || {
                format!(
                    "a key of rate card {} has no pattern per dimension",
                    card.code
                )
            },
        )?;
    }
    for plan in &scenario.plans {
        for entitlement in &plan.entitlements {
            let code = entitlement.feature_code();
            ensure(features.contains(code), || {
                format!("plan {} entitles an unknown feature {code}", plan.name)
            })?;
        }
        for line in plan.pricing_page.iter().flat_map(|page| &page.highlights) {
            if let HighlightSpec::Feature(code) = line {
                ensure(features.contains(code), || {
                    format!("plan {} highlights an unknown feature {code}", plan.name)
                })?;
            }
        }
        let priced = plan
            .components
            .iter()
            .chain(plan.currency_prices.iter().flat_map(|p| &p.components));
        for component in priced {
            if let FeeType::RateCard {
                metric_code,
                rate_card,
                ..
            } = &component.fee
            {
                ensure(
                    scenario.rate_cards.iter().any(|card| {
                        card.code == rate_card.as_str() && card.metric_code == metric_code.as_str()
                    }),
                    || {
                        format!(
                            "{} of plan {} is rated by no rate card {rate_card} on {metric_code}",
                            component.name, plan.name
                        )
                    },
                )?;
            }
        }
        for component in plan.components.iter().chain(plan.revision.iter().flatten()) {
            if let Some(code) = component.fee.metric_code() {
                ensure(metrics.contains(code.as_str()), || {
                    format!(
                        "{} of plan {} prices an unknown metric {code}",
                        component.name, plan.name
                    )
                })?;
            }
            if let FeeType::Credit {
                credit_unit: Some(unit),
                ..
            }
            | FeeType::Usage {
                credit_unit:
                    Some(UnitPricing {
                        credit_unit: unit, ..
                    }),
                ..
            } = &component.fee
            {
                ensure(units.contains(unit.as_str()), || {
                    format!(
                        "{} of plan {} uses an unknown credit unit {unit}",
                        component.name, plan.name
                    )
                })?;
            }
        }
    }

    for plan in &scenario.plans {
        let mut currencies = HashSet::from([plan.currency.as_str()]);
        for prices in &plan.currency_prices {
            ensure(currencies.insert(prices.currency), || {
                format!("plan {} prices {} twice", plan.name, prices.currency)
            })?;
            for component in &prices.components {
                ensure(
                    plan.components.iter().any(|c| c.name == component.name),
                    || {
                        format!(
                            "plan {} prices {} in {}, not one of its components",
                            plan.name, component.name, prices.currency
                        )
                    },
                )?;
            }
        }
    }

    check_coupons(scenario, now)?;
    check_customer_properties(scenario)?;

    for customer in &scenario.customers {
        for sub in &customer.subscriptions {
            // A subscription bills in its customer's currency: the plan must be priced in it.
            if let Some(plan) = scenario.plans.iter().find(|p| p.name == sub.plan_name) {
                ensure(
                    plan.currency == customer.currency
                        || plan
                            .currency_prices
                            .iter()
                            .any(|p| p.currency == customer.currency),
                    || {
                        format!(
                            "{} pays in {}, plan {} is not priced in it",
                            customer.name, customer.currency, plan.name
                        )
                    },
                )?;
                // Options are named after their cadence; a plan of none has its default only.
                if let Some(alias) = sub.option {
                    let sold = if plan.cadence_options.is_empty() {
                        alias == meteroid_store::domain::plan_options::DEFAULT_OPTION_ALIAS
                    } else {
                        plan.cadence_options
                            .iter()
                            .any(|cadence| format!("{cadence:?}").eq_ignore_ascii_case(alias))
                    };
                    ensure(sold, || {
                        format!(
                            "{} buys option {alias} of plan {}, which has none so named",
                            customer.name, plan.name
                        )
                    })?;
                }
            }
            for component in &sub.overrides {
                ensure(
                    scenario.plans.iter().any(|p| {
                        p.name == sub.plan_name
                            && p.components.iter().any(|c| c.name == component.name)
                    }),
                    || {
                        format!(
                            "{} overrides {}, not a component of plan {}",
                            customer.name, component.name, sub.plan_name
                        )
                    },
                )?;
                if let FeeType::Usage {
                    credit_unit: Some(UnitPricing { credit_unit, .. }),
                    ..
                } = &component.fee
                {
                    ensure(units.contains(credit_unit.as_str()), || {
                        format!(
                            "{} prices {} in an unknown credit unit {credit_unit}",
                            customer.name, component.name
                        )
                    })?;
                }
            }
        }
    }

    let as_of = credit_traffic::seed_cutoff(now);
    let clock = Clock::new(as_of, 0);
    for customer in scenario.customers.iter().filter(|c| !c.credit.is_empty()) {
        ensure(customer.history.is_none(), || {
            format!("{} has both a history and credit of its own", customer.name)
        })?;
        let times: Vec<NaiveDateTime> = customer
            .credit
            .iter()
            .map(|e| clock.resolve(e.at))
            .collect();
        ensure(
            times.windows(2).all(|pair| pair[0] <= pair[1]) && times.iter().all(|at| *at < as_of),
            || {
                format!(
                    "{}'s credit is not in time order before the seed",
                    customer.name
                )
            },
        )?;
        for event in &customer.credit {
            ensure(account_credit::is_account_credit(&event.action), || {
                format!("{}'s credit cannot {:?}", customer.name, event.action)
            })?;
            if let Action::RedeemPromoCode { code } = event.action {
                ensure(scenario.promo_codes.iter().any(|p| p.code == code), || {
                    format!("{} redeems an unknown promotion code {code}", customer.name)
                })?;
            }
        }
    }

    let Some(program) = &scenario.credits else {
        return Ok(());
    };
    ensure(metrics.contains(program.usage_metric_code.as_str()), || {
        format!(
            "credit usage is metered by an unknown metric {}",
            program.usage_metric_code
        )
    })?;
    let mut ids = HashMap::new();
    for customer in &scenario.customers {
        let Some(history) = &customer.history else {
            continue;
        };
        let as_of = credit_traffic::seed_cutoff(now);
        let events = resolve_events(history, &Clock::new(as_of, history.cycle_age_days));
        ensure(
            events.windows(2).all(|pair| pair[0].0 <= pair[1].0)
                && events.iter().all(|(at, _)| *at < as_of),
            || {
                format!(
                    "{}'s history is not in time order before the seed",
                    customer.name
                )
            },
        )?;
        ensure(
            credits::subscribed_plan(customer, &scenario.plans).is_some(),
            || format!("{} subscribes to no plan of the scenario", customer.name),
        )?;
        for event in &history.events {
            if let Action::RedeemPromoCode { code } = event.action {
                ensure(scenario.promo_codes.iter().any(|p| p.code == code), || {
                    format!("{} redeems an unknown promotion code {code}", customer.name)
                })?;
            }
        }
        ids.insert(customer.name.clone(), CustomerId::new());
    }
    // Every usage target must be reachable exactly at list price.
    credits::history_usage(
        &scenario.customers,
        &scenario.plans,
        &scenario.credit_units,
        &ids,
        credit_traffic::seed_cutoff(now),
    )?;
    Ok(())
}

/// Each coupon is on plans of the scenario and redeemable at the seed; each one applied is
/// on a plan in its scope, by a customer its rule lets in, within its redemption limit.
fn check_coupons(scenario: &Scenario, now: NaiveDateTime) -> StoreResult<()> {
    let today = now.date();
    let mut codes = HashSet::new();
    for coupon in &scenario.coupons {
        ensure(codes.insert(coupon.code), || {
            format!("coupon {} is defined twice", coupon.code)
        })?;
        for plan in &coupon.plans {
            ensure(scenario.plans.iter().any(|p| p.name == *plan), || {
                format!("coupon {} is on an unknown plan {plan}", coupon.code)
            })?;
        }
        ensure(
            coupon.valid_from.is_none_or(|from| from <= today)
                && coupon.expires_on.is_none_or(|until| until > today),
            || format!("coupon {} is not redeemable at the seed", coupon.code),
        )?;
    }

    let mut redeemed: HashMap<&str, u32> = HashMap::new();
    for customer in &scenario.customers {
        for sub in &customer.subscriptions {
            for code in &sub.coupons {
                let coupon = scenario
                    .coupons
                    .iter()
                    .find(|p| p.code == *code)
                    .ok_or_else(|| {
                        StoreError::InvalidArgument(format!(
                            "{} redeems an unknown coupon {code}",
                            customer.name
                        ))
                    })?;
                ensure(
                    coupon.plans.is_empty() || coupon.plans.contains(&sub.plan_name.as_str()),
                    || {
                        format!(
                            "{} redeems {code} on {}, not one of its plans",
                            customer.name, sub.plan_name
                        )
                    },
                )?;
                if let Some(rule) = &coupon.eligibility {
                    let verdict = check(rule, &facts_of(customer));
                    ensure(verdict.is_ok(), || {
                        format!("{} may not redeem {code}", customer.name)
                    })?;
                }
                *redeemed.entry(*code).or_default() += 1;
            }
        }
    }
    for coupon in &scenario.coupons {
        let count = redeemed.get(coupon.code).copied().unwrap_or(0);
        ensure(
            coupon.redemption_limit.is_none_or(|limit| count <= limit),
            || format!("coupon {} is redeemed past its limit", coupon.code),
        )?;
    }
    Ok(())
}

/// Each customer's custom properties are defined by the scenario, one with options holding one;
/// a plan is for customers holding defined ones.
fn check_customer_properties(scenario: &Scenario) -> StoreResult<()> {
    for plan in &scenario.plans {
        let wanted = plan.eligibility.iter().flat_map(|rule| &rule.properties);
        for property in wanted {
            ensure(
                scenario
                    .customer_properties
                    .iter()
                    .any(|p| p.key == property.key),
                || {
                    format!(
                        "plan {} is for an undefined customer property {}",
                        plan.name, property.key
                    )
                },
            )?;
        }
    }
    for customer in &scenario.customers {
        let Some(held) = customer.custom_properties.as_object() else {
            continue;
        };
        for (key, value) in held {
            let defined = scenario.customer_properties.iter().find(|p| p.key == key);
            ensure(defined.is_some(), || {
                format!(
                    "{} holds an undefined customer property {key}",
                    customer.name
                )
            })?;
            let options = defined.map(|p| p.options.as_slice()).unwrap_or_default();
            let one_of = value.as_str().is_some_and(|v| options.contains(&v));
            ensure(options.is_empty() || one_of, || {
                format!("{}'s {key} is not one of its options", customer.name)
            })?;
        }
    }
    Ok(())
}

/// What an eligibility rule reads of a scenario customer, as of the customer it seeds.
fn facts_of(customer: &Customer) -> CustomerFacts {
    let country = customer
        .billing_address
        .as_ref()
        .and_then(|address| address.country.as_ref())
        .map(|country| country.code.clone());
    CustomerFacts::new(country, &customer.custom_properties)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::scenarios::{ai_credits, ai_operator, currencies, pulse};

    #[test]
    fn every_sandbox_passes() {
        let now = chrono::Utc::now().naive_utc();
        check_scenario(&pulse::pulse_scenario(now.date()), now).unwrap();
        check_scenario(&ai_credits::ai_credits_scenario(), now).unwrap();
        check_scenario(&currencies::currencies_scenario(now.date()), now).unwrap();
        check_scenario(&ai_operator::ai_operator_scenario(now.date()), now).unwrap();
    }

    #[test]
    fn a_customer_is_sold_a_plan_priced_in_its_currency() {
        let now = chrono::Utc::now().naive_utc();
        let mut scenario = currencies::currencies_scenario(now.date());
        for plan in &mut scenario.plans {
            plan.currency_prices.clear();
        }
        let refused = check_scenario(&scenario, now).unwrap_err();
        assert!(format!("{refused:?}").contains("is not priced in it"));
    }

    #[test]
    fn a_coupon_applied_must_exist() {
        let now = chrono::Utc::now().naive_utc();
        let mut scenario = currencies::currencies_scenario(now.date());
        scenario.coupons.clear();
        let refused = check_scenario(&scenario, now).unwrap_err();
        assert!(format!("{refused:?}").contains("unknown coupon"));
    }

    #[test]
    fn a_coupon_is_redeemed_only_by_who_its_rule_lets_in() {
        let now = chrono::Utc::now().naive_utc();
        let mut scenario = currencies::currencies_scenario(now.date());
        for customer in &mut scenario.customers {
            if customer.currency == "BRL" {
                customer.custom_properties = serde_json::Value::Null;
            }
        }
        let refused = check_scenario(&scenario, now).unwrap_err();
        assert!(format!("{refused:?}").contains("may not redeem"));
    }

    #[test]
    fn a_coupon_is_redeemed_on_its_plans_only() {
        let now = chrono::Utc::now().naive_utc();
        let mut scenario = currencies::currencies_scenario(now.date());
        for coupon in &mut scenario.coupons {
            coupon.plans = vec!["Business"];
        }
        let refused = check_scenario(&scenario, now).unwrap_err();
        assert!(format!("{refused:?}").contains("not one of its plans"));
    }

    #[test]
    fn a_customer_property_must_be_defined() {
        let now = chrono::Utc::now().naive_utc();
        let mut scenario = currencies::currencies_scenario(now.date());
        scenario.customer_properties.clear();
        let refused = check_scenario(&scenario, now).unwrap_err();
        assert!(format!("{refused:?}").contains("undefined customer property student"));
    }

    #[test]
    fn a_promotion_code_must_exist() {
        let now = chrono::Utc::now().naive_utc();
        let mut scenario = ai_credits::ai_credits_scenario();
        scenario.promo_codes.clear();
        let refused = check_scenario(&scenario, now).unwrap_err();
        assert!(format!("{refused:?}").contains("unknown promotion code"));
    }
}
