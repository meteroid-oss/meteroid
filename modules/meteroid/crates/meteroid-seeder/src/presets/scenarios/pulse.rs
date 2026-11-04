//! The default sandbox business: **Pulse**, a team-messaging & collaboration
//! platform (a Slack-style SaaS). Deliberately one coherent business rather than
//! a grab-bag of archetypes: a single currency (EUR), three plans and fourteen
//! customers chosen to exercise the whole billing surface while staying legible.
//!
//! Pricing mixes the primitives a real seat-based SaaS uses:
//! - per-seat pricing (a slot component) on every plan,
//! - metered AI usage (arrears, per-unit) on the paid tiers, capped per billing
//!   cycle by an "AI Assist Actions" entitlement (overage allowed and billed),
//! - an enterprise platform fee plus a one-time onboarding charge,
//! - two attachable add-ons (security, support).
//!
//! The customer base spans the last 12 months and tells a readable story — a
//! long-lived enterprise anchor, one upgrade, one downgrade, one churn, a run of
//! new logos, and two accounts that moved in the last few weeks (one expanding
//! its seats, one contracting) — so New Business, Churn, usage revenue, the AR
//! aging, and the 30-day MRR-momentum board all have something to show.
//! Acquisition is weighted toward the recent months (more, larger wins lately,
//! including a second enterprise) so the MRR curve reads as accelerating growth
//! rather than a flat line, and start dates are day-level (not clean month
//! boundaries) so the steps are irregular. The churn and downgrade sit in the
//! middle of the window so the most recent movements read as growth.
//!
//! A few accounts carry credit: a prepaid commitment the anchor draws down, a welcome credit on a
//! fresh signup, an AI credit pack with automatic top-up, and a usage budget.
//!
//! Every built-in account signal has a staged example in the current cycle (limit, collapse,
//! minimum, renewal, a failed payment, and both enterprise accounts left on Enterprise v1 once v2
//! raises the platform fee); sandbox maintenance leaves the usage of [`STAGED_USAGE_CUSTOMERS`]
//! alone.

use super::common::{add_on, customer, metric, org, rate, seats, usage};
use super::domain::{
    AutoTopUpAmount, AutoTopUpSpec, BudgetSpec, CardBehavior, CardSpec, CreditUnitSpec, Customer,
    FeatureKind, FeatureSpec, FeeType, Plan, PriceComponent, PromoCodeSpec, QuoteSpec, Scenario,
    Subscription, UnitPricing,
};
use super::timeline::{Action, Event, When};
use chrono::{Datelike, Days, Duration, Months, NaiveDate};
use meteroid_store::domain::credit_exhausted::UsageAtZero;
use meteroid_store::domain::{
    BillingMetricAggregateEnum, BillingPeriodEnum, PlanTypeEnum, UsagePricingModel,
};
use rust_decimal_macros::dec;

const STARTER: &str = "Starter";
const SCALE: &str = "Scale";
const ENTERPRISE: &str = "Enterprise";

const AI_USAGE: &str = "AI Usage";

const SECURITY: &str = "Advanced Security & Compliance";
const SUPPORT: &str = "Premium Support";

const CURRENCY: &str = "EUR";

const AI_ACTIONS: &str = "ai_actions";

const AI_CREDITS: &str = "AI_CREDITS";
const WELCOME_CODE: &str = "WELCOME250";

/// Customers whose current-cycle usage is staged for a detector; regenerating their
/// usage would erase the staged outcome.
pub const STAGED_USAGE_CUSTOMERS: &[&str] = &[
    "Cedar Foundry",
    "Batchline GmbH",
    "Pinewood Interactive",
    "Northwind Collective",
];

fn plan(name: &str, components: Vec<PriceComponent>) -> Plan {
    Plan::new(name, CURRENCY, PlanTypeEnum::Standard, components)
}

/// One-time onboarding + fixed platform fee + per-seat + metered AI at a preferential rate.
fn enterprise_components(platform_fee: rust_decimal::Decimal) -> Vec<PriceComponent> {
    vec![
        PriceComponent {
            name: "Onboarding".to_string(),
            fee: FeeType::OneTime {
                unit_price: dec!(2500),
                quantity: 1,
            },
        },
        rate("Platform Fee", platform_fee),
        seats(dec!(22)),
        usage(AI_USAGE, AI_ACTIONS, dec!(0.015)),
    ]
}

/// AI usage drawn from AI credits, one per action; past the credit, billed at the Scale rate.
fn ai_usage_in_credits() -> PriceComponent {
    PriceComponent {
        name: AI_USAGE.to_string(),
        fee: FeeType::Usage {
            metric_code: AI_ACTIONS.to_string(),
            pricing: UsagePricingModel::PerUnit { rate: dec!(1) },
            cadence: BillingPeriodEnum::Monthly,
            credit_unit: Some(UnitPricing {
                credit_unit: AI_CREDITS.to_string(),
                rate: dec!(0.02),
            }),
        },
    }
}

fn days_ago(days: i64) -> When {
    When::Ago(Duration::days(days))
}

pub fn pulse_scenario(now: NaiveDate) -> Scenario {
    Scenario {
        name: "Pulse".to_string(),
        metrics: vec![metric(
            AI_ACTIONS,
            "AI Assist Actions",
            BillingMetricAggregateEnum::Sum,
            "value",
        )],
        features: vec![FeatureSpec {
            code: AI_ACTIONS.to_string(),
            name: "AI Assist Actions".to_string(),
            kind: FeatureKind::Metered {
                metric_code: AI_ACTIONS.to_string(),
                unlimited_by_default: false,
            },
        }],
        plans: vec![
            // Starter: pure per-seat, for small teams.
            plan(STARTER, vec![seats(dec!(8))]),
            // Scale: per-seat plus metered AI usage. "Seats" and "AI Usage" reuse
            // the same products as the other plans, at their own price points.
            plan(
                SCALE,
                vec![seats(dec!(14)), usage(AI_USAGE, AI_ACTIONS, dec!(0.02))],
            )
            .with_cycle_limit(AI_ACTIONS, dec!(20000), 80),
            // Enterprise: v2 raised the platform fee; the existing accounts stay on v1.
            plan(ENTERPRISE, enterprise_components(dec!(1500)))
                .with_cycle_limit(AI_ACTIONS, dec!(150000), 85)
                .with_revision(enterprise_components(dec!(1800))),
        ],
        add_ons: vec![add_on(SECURITY, dec!(400)), add_on(SUPPORT, dec!(600))],
        customers: customers(now),
        organization: Some(org("FR21349824531", "Paris", "75011")),
        quotes: vec![
            // A signed deal that converted into a subscription (new logo), plus an
            // open upsell quote to an existing account.
            QuoteSpec {
                customer: "Harbor Digital".to_string(),
                plan_name: SCALE.to_string(),
                convert: true,
            },
            QuoteSpec {
                customer: "Batchline GmbH".to_string(),
                plan_name: ENTERPRISE.to_string(),
                convert: false,
            },
        ],
        credits: None,
        // Sold below the €0.02 an action costs past the credit.
        credit_units: vec![CreditUnitSpec {
            code: AI_CREDITS,
            name: "AI credit",
            plural_name: "AI credits",
            symbol: None,
            precision: 0,
            selling_prices: vec![(CURRENCY, dec!(0.015))],
        }],
        promo_codes: vec![PromoCodeSpec {
            code: WELCOME_CODE,
            name: "Welcome credit for new teams",
            amount: dec!(250),
            currency: CURRENCY,
            expires_after_days: 90,
        }],
        reporting_currency: None,
    }
}

fn customers(now: NaiveDate) -> Vec<Customer> {
    // Day-level offsets from `now` (not clean month boundaries) so the MRR steps are
    // irregular. Acquisition is deliberately back-weighted: a handful of early
    // accounts, then a denser, larger run of recent wins, so cumulative MRR bends
    // upward toward today instead of tracing a straight line. Churn and the
    // downgrade sit mid-window so the most recent movements read as growth.
    let ago = |days: u64| now.checked_sub_days(Days::new(days)).unwrap_or(now);
    let next_day = |date: NaiveDate| date.checked_add_days(Days::new(1)).unwrap_or(date);
    let in_days = |days: u64| now.checked_add_days(Days::new(days)).unwrap_or(now);
    // A few days ahead, kept inside this month while it has days left: the month's targets then
    // have changes on the calendar to read.
    let month_end = now
        .with_day(1)
        .and_then(|first| first.checked_add_months(Months::new(1)))
        .and_then(|next| next.pred_opt())
        .unwrap_or(now);
    let before_close = |days: u64| {
        let day = in_days(days);
        if day > month_end && month_end > now {
            month_end
        } else {
            day
        }
    };

    let meadowlark_churn = ago(212); // ~7 months
    let fjord_churn = ago(120); // ~4 months

    vec![
        // Long-lived enterprise anchor near the start of the 12-month window — but it
        // has gone dark: high MRR (Enterprise, 120 seats) with zero product usage, so
        // it surfaces as a dormant-whale / shelfware adoption risk, and its €800 AI
        // Usage commitment will be billed as a minimum shortfall.
        customer(
            "Northwind Collective",
            CURRENCY,
            "DE",
            "Berlin",
            vec![
                Subscription::active(ENTERPRISE, ago(372))
                    .with_seats(120)
                    .with_add_on(SECURITY, 1)
                    .with_minimum(Some(AI_USAGE), dec!(800)),
            ],
        )
        .without_usage()
        // Prepaid a commitment by bank transfer; every invoice since draws it down.
        .with_credit(vec![
            Event::new(
                days_ago(80),
                Action::Purchase {
                    purchase: "commitment",
                    denomination: CURRENCY,
                    amount: dec!(25000),
                    expires_in_months: Some(12),
                },
            ),
            Event::new(
                days_ago(80).plus_minutes(60),
                Action::BankTransfer {
                    payable: "commitment",
                },
            ),
        ]),
        // Mid-market account on Scale that just expanded (45 → 70 seats ~18 days
        // ago) — a recent, in-window MRR increase so Momentum has a live "Expanding".
        // It also carries an open enterprise upsell quote, and has already burnt
        // through its AI limit this cycle (overage billed at €0.02).
        customer(
            "Batchline GmbH",
            CURRENCY,
            "DE",
            "Munich",
            vec![
                Subscription::churned(SCALE, ago(340), ago(18)).with_seats(45),
                Subscription::active(SCALE, next_day(ago(18))).with_seats(70),
            ],
        )
        .with_cycle_usage(AI_ACTIONS, 118),
        // Downgrade mid-window (Scale -> Starter), so it isn't a recent move; the
        // Starter subscription is now cancelled effective in 20 days.
        customer(
            "Meadowlark Labs",
            CURRENCY,
            "FR",
            "Lyon",
            vec![
                Subscription::churned(SCALE, ago(310), meadowlark_churn).with_seats(30),
                Subscription::ending(STARTER, next_day(meadowlark_churn), in_days(20))
                    .with_seats(12),
            ],
        ),
        // Churn in the middle of the window.
        customer(
            "Tin Roof Software",
            CURRENCY,
            "ES",
            "Madrid",
            vec![
                Subscription::churned(SCALE, ago(285), ago(150))
                    .with_seats(25)
                    .with_add_on(SUPPORT, 1),
            ],
        ),
        // Upgrade mid-window: outgrew Starter and moved to Scale with more seats.
        customer(
            "Fjord Studio",
            CURRENCY,
            "FR",
            "Paris",
            vec![
                Subscription::churned(STARTER, ago(250), fjord_churn).with_seats(15),
                Subscription::active(SCALE, next_day(fjord_churn))
                    .with_seats(40)
                    .with_override(ai_usage_in_credits()),
            ],
        )
        // Moving to Scale, it bought an AI credit pack; a card tops it up automatically.
        .with_credit(fjord_ai_credits(119)),
        // --- Recent, accelerating run of new logos (last ~6 months) ---
        // Heavy AI user early in its cycle: already past its 80 % warning.
        customer(
            "Cedar Foundry",
            CURRENCY,
            "BE",
            "Antwerp",
            vec![Subscription::active(SCALE, ago(188)).with_seats(18)],
        )
        .with_cycle_usage(AI_ACTIONS, 92)
        // Its staged AI usage (~€370) is past 80 % of this budget.
        .with_credit(vec![Event::new(
            days_ago(30),
            Action::SetBudget(BudgetSpec {
                amount: dec!(450),
                hard: false,
            }),
        )]),
        // Two of the recent self-serve logos landed through a 14-day trial, so the
        // trial step of the flat-rate lens has a real conversion to show.
        customer(
            "Vantage Labs",
            CURRENCY,
            "PT",
            "Lisbon",
            vec![
                Subscription::active(STARTER, ago(156))
                    .with_seats(9)
                    .with_trial(14),
            ],
        ),
        customer(
            "Ashby Foods",
            CURRENCY,
            "IT",
            "Milan",
            vec![
                Subscription::active(SCALE, ago(118))
                    .with_seats(22)
                    .with_trial(14),
            ],
        ),
        // A second enterprise win — a large, recent step up in MRR.
        customer(
            "Northgate Systems",
            CURRENCY,
            "DE",
            "Hamburg",
            vec![
                Subscription::active(ENTERPRISE, ago(92))
                    .with_seats(60)
                    .with_add_on(SUPPORT, 1),
            ],
        ),
        // Account that contracted recently (24 → 14 seats ~20 days ago) after two
        // months on Scale — a live in-window MRR decline so Momentum has a "Slipping".
        customer(
            "Larkfield Media",
            CURRENCY,
            "IE",
            "Dublin",
            vec![
                Subscription::churned(SCALE, ago(63), ago(20)).with_seats(24),
                Subscription::active(SCALE, next_day(ago(20))).with_seats(14),
            ],
        ),
        // After a full first cycle, its AI usage pace dropped to a fifth.
        customer(
            "Pinewood Interactive",
            CURRENCY,
            "NL",
            "Rotterdam",
            vec![Subscription::active(SCALE, ago(45)).with_seats(16)],
        )
        .with_usage_collapse(0.2),
        customer(
            "Solstice Retail",
            CURRENCY,
            "FR",
            "Nantes",
            vec![Subscription::active(STARTER, ago(21)).with_seats(11)],
        ),
        // Fresh signup on a 14-day trial (started ~10 days ago, still in trial).
        customer(
            "Kestrel Analytics",
            CURRENCY,
            "NL",
            "Amsterdam",
            vec![
                Subscription::active(SCALE, ago(10))
                    .with_seats(20)
                    .with_trial(14),
            ],
        )
        // Its first invoice, once the trial ends, draws the welcome credit.
        .with_credit(vec![Event::new(
            days_ago(10),
            Action::RedeemPromoCode { code: WELCOME_CODE },
        )]),
        // Prospect won via a signed quote — its subscription is created on conversion (recent).
        customer("Harbor Digital", CURRENCY, "DE", "Berlin", vec![]),
        // Signed last week, starting before the month closes: new business already on the
        // calendar.
        customer(
            "Halvorsen Freight",
            CURRENCY,
            "NL",
            "Utrecht",
            vec![Subscription::active(SCALE, before_close(2)).with_seats(20)],
        ),
        // Gave notice: its cancellation takes effect before the month closes.
        customer(
            "Orla Studio",
            CURRENCY,
            "IE",
            "Cork",
            vec![Subscription::ending(STARTER, ago(140), before_close(4)).with_seats(9)],
        ),
    ]
}

fn fjord_ai_credits(switched_days_ago: i64) -> Vec<Event> {
    let switched = days_ago(switched_days_ago);
    vec![
        Event::new(
            switched,
            Action::AddCard {
                card: "visa",
                spec: CardSpec {
                    brand: "visa",
                    last4: "4242",
                    behavior: CardBehavior::Approved,
                    expires_in_months: 30,
                },
                make_default: true,
            },
        ),
        Event::new(
            switched.plus_minutes(30),
            Action::Purchase {
                purchase: "pack",
                denomination: AI_CREDITS,
                amount: dec!(50000),
                expires_in_months: None,
            },
        ),
        Event::new(
            switched.plus_minutes(60),
            Action::BankTransfer { payable: "pack" },
        ),
        Event::new(
            switched.plus_minutes(90),
            Action::ConfigureWallet {
                denomination: AI_CREDITS,
                low_balance_thresholds: vec![dec!(15000)],
                usage_at_zero: UsageAtZero::NeverPause,
                auto_top_up: Some(AutoTopUpSpec {
                    threshold: dec!(10000),
                    amount: AutoTopUpAmount::Fixed(dec!(25000)),
                    daily_cap: None,
                    monthly_cap: Some(dec!(50000)),
                }),
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::scenarios::domain::{MinimumSpec, PlanEntitlementSpec, UsageProfile};

    #[test]
    fn pulse_scenario_stages_every_builtin_detector() {
        let now = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let scenario = pulse_scenario(now);
        let customer = |name: &str| scenario.customers.iter().find(|c| c.name == name).unwrap();
        let plan = |name: &str| scenario.plans.iter().find(|p| p.name == name).unwrap();

        assert_eq!(
            scenario.features,
            vec![FeatureSpec {
                code: AI_ACTIONS.to_string(),
                name: "AI Assist Actions".to_string(),
                kind: FeatureKind::Metered {
                    metric_code: AI_ACTIONS.to_string(),
                    unlimited_by_default: false,
                },
            }]
        );
        let limit = |plan_name: &str| plan(plan_name).entitlement(AI_ACTIONS).cloned();
        let spec = |limit, warning_pct| PlanEntitlementSpec::Metered {
            feature_code: AI_ACTIONS.to_string(),
            limit,
            warning_pct,
            block: None,
        };
        assert_eq!(limit(SCALE), Some(spec(dec!(20000), 80)));
        assert_eq!(limit(ENTERPRISE), Some(spec(dec!(150000), 85)));
        assert_eq!(limit(STARTER), None);
        assert_eq!(
            plan(ENTERPRISE).revision.as_ref().map(Vec::len),
            Some(plan(ENTERPRISE).components.len())
        );
        assert!(plan(SCALE).revision.is_none() && plan(STARTER).revision.is_none());

        let cycle_usage = |pct_of_limit| UsageProfile::CycleUsage {
            metric_code: AI_ACTIONS.to_string(),
            pct_of_limit,
        };
        assert_eq!(customer("Cedar Foundry").usage, cycle_usage(92));
        assert_eq!(customer("Batchline GmbH").usage, cycle_usage(118));
        assert_eq!(
            customer("Pinewood Interactive").usage,
            UsageProfile::Collapse { keep_ratio: 0.2 }
        );

        let northwind = customer("Northwind Collective");
        assert_eq!(northwind.usage, UsageProfile::None);
        let northwind_sub = northwind.current_subscription(now).unwrap();
        assert_eq!(northwind_sub.plan_name, ENTERPRISE);
        assert_eq!(
            northwind_sub.minimum,
            Some(MinimumSpec {
                product_name: Some(AI_USAGE.to_string()),
                amount: dec!(800),
            })
        );

        let meadowlark = customer("Meadowlark Labs")
            .current_subscription(now)
            .unwrap();
        assert_eq!(meadowlark.plan_name, STARTER);
        assert_eq!(meadowlark.end_date, NaiveDate::from_ymd_opt(2026, 10, 13));

        let mut staged: Vec<&str> = scenario
            .customers
            .iter()
            .filter(|c| c.has_staged_usage())
            .map(|c| c.name.as_str())
            .collect();
        let mut expected = STAGED_USAGE_CUSTOMERS.to_vec();
        staged.sort_unstable();
        expected.sort_unstable();
        assert_eq!(staged, expected);
        assert_eq!(scenario.customers.len(), 16);

        // The month closes on the 30th: both calendar changes land before it.
        let halvorsen = &customer("Halvorsen Freight").subscriptions[0];
        assert_eq!(
            halvorsen.start_date,
            NaiveDate::from_ymd_opt(2026, 9, 25).unwrap()
        );
        let orla = &customer("Orla Studio").subscriptions[0];
        assert_eq!(orla.end_date, NaiveDate::from_ymd_opt(2026, 9, 27));
        let late = pulse_scenario(NaiveDate::from_ymd_opt(2026, 9, 29).unwrap());
        let orla_late = late
            .customers
            .iter()
            .find(|c| c.name == "Orla Studio")
            .unwrap();
        assert_eq!(
            orla_late.subscriptions[0].end_date,
            NaiveDate::from_ymd_opt(2026, 9, 30),
            "kept inside the month"
        );
    }

    #[test]
    fn pulse_scenario_gives_four_accounts_credit() {
        let scenario = pulse_scenario(NaiveDate::from_ymd_opt(2026, 9, 23).unwrap());
        let with_credit: Vec<&str> = scenario
            .customers
            .iter()
            .filter(|c| !c.credit.is_empty())
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            with_credit,
            [
                "Northwind Collective",
                "Fjord Studio",
                "Cedar Foundry",
                "Kestrel Analytics"
            ]
        );
        let fjord = scenario
            .customers
            .iter()
            .find(|c| c.name == "Fjord Studio")
            .unwrap();
        let scale = &fjord.subscriptions[1];
        assert_eq!(scale.plan_name, SCALE);
        assert!(matches!(
            &scale.overrides[..],
            [PriceComponent {
                fee: FeeType::Usage {
                    credit_unit: Some(UnitPricing { credit_unit, .. }),
                    ..
                },
                ..
            }] if credit_unit == AI_CREDITS
        ));
    }
}
