//! What a scenario must satisfy before its tenant is created, checked synchronously so a sandbox
//! never starts seeding something it cannot finish.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, CustomerId};
use meteroid_store::StoreResult;
use meteroid_store::errors::StoreError;

use crate::presets::scenarios::domain::{FeeType, Scenario, UnitPricing};
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
    }
    for plan in &scenario.plans {
        for entitlement in &plan.entitlements {
            let code = entitlement.feature_code();
            ensure(features.contains(code), || {
                format!("plan {} entitles an unknown feature {code}", plan.name)
            })?;
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

    for customer in &scenario.customers {
        for sub in &customer.subscriptions {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::scenarios::{ai_credits, pulse};

    #[test]
    fn both_sandboxes_pass() {
        let now = chrono::Utc::now().naive_utc();
        check_scenario(&pulse::pulse_scenario(now.date()), now).unwrap();
        check_scenario(&ai_credits::ai_credits_scenario(), now).unwrap();
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
