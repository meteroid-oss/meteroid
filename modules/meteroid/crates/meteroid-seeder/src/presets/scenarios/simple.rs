use super::domain::*;
use chrono::NaiveDate;
use meteroid_store::domain::{BillingPeriodEnum, TermRate};

use rust_decimal_macros::dec;

const STANDARD_20_PLAN: &str = "Standard 20 Plan";
const STANDARD_80_PLAN: &str = "Standard 80 Plan";

pub fn basic_scenario_1() -> Scenario {
    Scenario {
        name: "Basic 1".to_string(),
        metrics: vec![],
        plans: vec![
            Plan {
                name: STANDARD_20_PLAN.to_string(),
                currency: "EUR".to_string(),
                plan_type: Default::default(),
                components: vec![PriceComponent {
                    name: "Base Price".to_string(),
                    fee: FeeType::Rate {
                        rates: vec![TermRate {
                            price: dec!(20),
                            term: BillingPeriodEnum::Monthly,
                        }],
                    },
                }],
            },
            Plan {
                name: STANDARD_80_PLAN.to_string(),
                currency: "EUR".to_string(),
                plan_type: Default::default(),
                components: vec![PriceComponent {
                    name: "Base Price".to_string(),
                    fee: FeeType::Rate {
                        rates: vec![TermRate {
                            price: dec!(80),
                            term: BillingPeriodEnum::Monthly,
                        }],
                    },
                }],
            },
        ],
        customers: vec![
            Customer {
                name: "Cobalt".to_string(),
                email: "billing@cobalt.oid".to_string(),
                currency: "EUR".to_string(),
                subscription: Subscription {
                    plan_name: STANDARD_20_PLAN.to_string(),
                    start_date: NaiveDate::from_ymd_opt(2024, 11, 3).unwrap(),
                },
            },
            Customer {
                name: "Pulse Analytics".to_string(),
                email: "billing@pulse.oid".to_string(),
                currency: "EUR".to_string(),
                subscription: Subscription {
                    plan_name: STANDARD_80_PLAN.to_string(),
                    start_date: NaiveDate::from_ymd_opt(2024, 1, 22).unwrap(),
                },
            },
            Customer {
                name: "Evergreen Dynamics".to_string(),
                email: "billing@evergreen.oid".to_string(),
                currency: "EUR".to_string(),
                subscription: Subscription {
                    plan_name: STANDARD_20_PLAN.to_string(),
                    start_date: NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
                },
            },
        ],
    }
}
