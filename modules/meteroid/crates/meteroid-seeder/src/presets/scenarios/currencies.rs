//! **Acme Analytics**, a product-analytics SaaS that sells one ladder of plans (Starter, Pro,
//! Business) in USD and EUR, as a Monthly and an Annual option (two months free), and Starter and Pro in
//! Brazilian reais, monthly only. Its customers subscribe in their own currency. Enterprise tops
//! the ladder, sold by talking to sales, its price kept off the pricing page. Students, a plan off
//! the ladder in EUR and BRL, is for customers whose `student` property is `verified`.
//!
//! Two coupons: "YC 2026" (code YC26), half off Pro for 12 months, and "Students in Brazil" (code
//! ESTUDANTE-BR), 40% off Starter and Pro for 6 months for customers in Brazil whose customer
//! custom property `student` is `verified`, set by the tenant after its own check.
//!
//! Seats come with three included; tracked events come with a million included each month,
//! billed per event past them. Pro and Business bill the LLM tokens their AI assistant uses by a
//! rate card per 1M tokens, by model and token type, Business at 20% off it.

use super::common::{customer, metric, org};
use super::domain::{
    CouponSpec, Customer, CustomerPropertySpec, EventProperty, FeatureKind, FeatureSpec, FeeType,
    HighlightSpec, Plan, PriceComponent, RateCardSpec, RateSpec, Scenario, Subscription, UnitLabel,
    UsageSample,
};
use BillingPeriodEnum::{Annual, Monthly};
use HighlightSpec::{Feature, Text};
use chrono::{Days, Months, NaiveDate};
use meteroid_store::domain::eligibility::{Eligibility, PropertyMatch};
use meteroid_store::domain::enums::TaxResolverEnum;
use meteroid_store::domain::{
    BillingMetricAggregateEnum, BillingPeriodEnum, DowngradePolicy, PlanTypeEnum, TermRate,
    TierRow, UpgradePolicy, UsagePricingModel,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

const USD: &str = "USD";
const EUR: &str = "EUR";
const BRL: &str = "BRL";

const STARTER: &str = "Starter";
const PRO: &str = "Pro";
const BUSINESS: &str = "Business";
const ENTERPRISE: &str = "Enterprise";
const STUDENTS: &str = "Students";

const PLATFORM_FEE: &str = "Platform fee";
const SEATS: &str = "Seats";
const EVENTS: &str = "Events";
const EVENTS_METRIC: &str = "tracked_events";
const LLM_TOKENS: &str = "LLM tokens";
const LLM_METRIC: &str = "llm_tokens";
const LLM_CARD: &str = "llm-tokens";
const MODEL: &str = "model";
const TOKEN_TYPE: &str = "token_type";
const PER_MILLION: Decimal = dec!(1000000);

const SEATS_INCLUDED: &str = "seats_included";
const SSO: &str = "sso";
const API_ACCESS: &str = "api_access";
const LLM_ALLOWANCE: &str = "llm_tokens_per_month";

const CONTACT_SALES: &str = "https://example.com/contact-sales";

const YC_CODE: &str = "YC26";
const STUDENTS_CODE: &str = "ESTUDANTE-BR";
const STUDENT_PROPERTY: &str = "student";
const VERIFIED: &str = "verified";
const PENDING: &str = "pending";

const MONTHLY: &str = "monthly";
const ANNUAL: &str = "annual";

const INCLUDED_SEATS: u32 = 3;
const INCLUDED_EVENTS: u64 = 1_000_000;
/// Business pays the LLM rate card's rates 20% off.
const BUSINESS_LLM_MULTIPLIER: Decimal = dec!(0.8);

/// A flat fee on each of `rates`' cadences.
fn platform_fee(rates: &[(BillingPeriodEnum, Decimal)]) -> PriceComponent {
    PriceComponent {
        name: PLATFORM_FEE.to_string(),
        fee: FeeType::Rate {
            rates: rates
                .iter()
                .map(|(term, price)| TermRate {
                    price: *price,
                    term: *term,
                })
                .collect(),
        },
    }
}

/// Per seat past the included ones, on each of `rates`' cadences.
fn seats(rates: &[(BillingPeriodEnum, Decimal)]) -> PriceComponent {
    PriceComponent {
        name: SEATS.to_string(),
        fee: FeeType::Slot {
            rates: rates
                .iter()
                .map(|(term, price)| TermRate {
                    price: *price,
                    term: *term,
                })
                .collect(),
            slot_unit_name: "seat".to_string(),
            upgrade_policy: UpgradePolicy::Prorated,
            downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
            minimum_count: Some(1),
            quota: None,
            included: Some(INCLUDED_SEATS),
        },
    }
}

/// A fee priced monthly and annually, a year costing ten months.
fn monthly_and_annual(monthly: Decimal) -> [(BillingPeriodEnum, Decimal); 2] {
    [(Monthly, monthly), (Annual, monthly * dec!(10))]
}

/// Tracked events past the monthly allowance, at `per_event`.
fn events(per_event: Decimal) -> PriceComponent {
    PriceComponent {
        name: EVENTS.to_string(),
        fee: FeeType::Usage {
            metric_code: EVENTS_METRIC.to_string(),
            pricing: UsagePricingModel::Tiered {
                tiers: vec![
                    TierRow {
                        first_unit: 0,
                        rate: Decimal::ZERO,
                        flat_fee: None,
                        flat_cap: None,
                    },
                    TierRow {
                        first_unit: INCLUDED_EVENTS,
                        rate: per_event,
                        flat_fee: None,
                        flat_cap: None,
                    },
                ],
                block_size: None,
            },
            cadence: Monthly,
            credit_unit: None,
        },
    }
}

/// Requests to the AI assistant, by model and token type, of 1k to 20k tokens each. `llama-3-70b`
/// has no rate of its own: the card's `*` row rates it.
fn llm_usage() -> UsageSample {
    UsageSample {
        min: 1_000,
        max: 20_000,
        properties: vec![
            EventProperty {
                key: MODEL,
                values: vec![
                    ("gpt-4o", 35),
                    ("gpt-4o-mini", 30),
                    ("claude-sonnet", 25),
                    ("llama-3-70b", 10),
                ],
            },
            EventProperty {
                key: TOKEN_TYPE,
                values: vec![("input", 75), ("output", 25)],
            },
        ],
    }
}

/// The LLM tokens the AI assistant uses, at the rate card's rates times `multiplier`.
fn llm_tokens(multiplier: Option<Decimal>) -> PriceComponent {
    PriceComponent {
        name: LLM_TOKENS.to_string(),
        fee: FeeType::RateCard {
            metric_code: LLM_METRIC.to_string(),
            rate_card: LLM_CARD.to_string(),
            multiplier,
            cadence: Monthly,
        },
    }
}

/// A key's rate per 1M tokens in USD, EUR and BRL.
fn llm_rate(
    model: &'static str,
    token_type: &'static str,
    label: &'static str,
    rates: [Decimal; 3],
) -> RateSpec {
    let [usd, eur, brl] = rates;
    RateSpec {
        key: vec![model, token_type],
        label: Some(label),
        rates: vec![(USD, usd), (EUR, eur), (BRL, brl)],
    }
}

/// Rates per 1M tokens by model and token type; anything no other key rates falls to `*`.
fn llm_rate_card(effective_from: NaiveDate) -> RateCardSpec {
    RateCardSpec {
        code: LLM_CARD,
        name: "LLM tokens",
        metric_code: LLM_METRIC,
        dimensions: vec![MODEL, TOKEN_TYPE],
        unit_size: PER_MILLION,
        effective_from,
        rates: vec![
            llm_rate(
                "gpt-4o",
                "input",
                "GPT-4o input",
                [dec!(2.50), dec!(2.30), dec!(13.50)],
            ),
            llm_rate(
                "gpt-4o",
                "output",
                "GPT-4o output",
                [dec!(10), dec!(9.20), dec!(54)],
            ),
            llm_rate(
                "claude-sonnet",
                "input",
                "Claude Sonnet input",
                [dec!(3), dec!(2.75), dec!(16.50)],
            ),
            llm_rate(
                "claude-sonnet",
                "output",
                "Claude Sonnet output",
                [dec!(15), dec!(13.80), dec!(81)],
            ),
            llm_rate(
                "gpt-4o-mini",
                "*",
                "GPT-4o mini",
                [dec!(0.60), dec!(0.55), dec!(3.25)],
            ),
            llm_rate("*", "*", "Anything else", [dec!(5), dec!(4.60), dec!(27)]),
        ],
    }
}

fn features() -> Vec<FeatureSpec> {
    let feature = |code: &str, name: &str, kind: FeatureKind, product: &'static str| FeatureSpec {
        code: code.to_string(),
        name: name.to_string(),
        kind,
        product: Some(product),
    };
    vec![
        feature(
            SEATS_INCLUDED,
            "Seats included",
            FeatureKind::Number {
                default: Decimal::from(INCLUDED_SEATS),
            },
            SEATS,
        ),
        feature(
            SSO,
            "SSO",
            FeatureKind::Boolean {
                default_enabled: false,
            },
            PLATFORM_FEE,
        ),
        feature(
            API_ACCESS,
            "API access",
            FeatureKind::Boolean {
                default_enabled: false,
            },
            PLATFORM_FEE,
        ),
        feature(
            LLM_ALLOWANCE,
            "LLM tokens per month",
            FeatureKind::Metered {
                metric_code: LLM_METRIC.to_string(),
                unlimited_by_default: false,
            },
            LLM_TOKENS,
        ),
    ]
}

/// Sold monthly or annually: an option of each, its fees billing on its cadence.
fn plan(name: &str, rank: i32, components: Vec<PriceComponent>) -> Plan {
    Plan::new(name, USD, PlanTypeEnum::Standard, components)
        .with_self_service_rank(rank)
        .with_cadence_options(&[Monthly, Annual])
}

fn plans() -> Vec<Plan> {
    vec![
        plan(
            STARTER,
            1,
            vec![
                platform_fee(&[(Monthly, dec!(29)), (Annual, dec!(290))]),
                seats(&monthly_and_annual(dec!(8))),
            ],
        )
        .with_currency_prices(
            EUR,
            vec![
                platform_fee(&[(Monthly, dec!(27)), (Annual, dec!(270))]),
                seats(&monthly_and_annual(dec!(7))),
            ],
        )
        .with_currency_prices(
            BRL,
            vec![
                platform_fee(&[(Monthly, dec!(149))]),
                seats(&[(Monthly, dec!(39))]),
            ],
        )
        .with_pricing_page(
            "For small teams shipping their first dashboards",
            false,
            vec![
                Feature(SEATS_INCLUDED),
                Text("Dashboards and funnels"),
                Text("Email support"),
            ],
        ),
        plan(
            PRO,
            2,
            vec![
                platform_fee(&[(Monthly, dec!(99)), (Annual, dec!(990))]),
                seats(&monthly_and_annual(dec!(12))),
                events(dec!(0.0004)),
                llm_tokens(None),
            ],
        )
        .with_currency_prices(
            EUR,
            vec![
                platform_fee(&[(Monthly, dec!(89)), (Annual, dec!(890))]),
                seats(&monthly_and_annual(dec!(11))),
                events(dec!(0.00036)),
                llm_tokens(None),
            ],
        )
        .with_currency_prices(
            BRL,
            vec![
                platform_fee(&[(Monthly, dec!(499))]),
                seats(&[(Monthly, dec!(59))]),
                events(dec!(0.002)),
                llm_tokens(None),
            ],
        )
        .with_feature(API_ACCESS, true)
        .with_cycle_limit(LLM_ALLOWANCE, dec!(25000000), 80)
        .with_pricing_page(
            "For product teams that live in their analytics",
            true,
            vec![
                Text("1M events a month included"),
                Feature(LLM_ALLOWANCE),
                Feature(API_ACCESS),
            ],
        ),
        plan(
            BUSINESS,
            3,
            vec![
                platform_fee(&[(Monthly, dec!(299)), (Annual, dec!(2990))]),
                seats(&monthly_and_annual(dec!(15))),
                events(dec!(0.0003)),
                llm_tokens(Some(BUSINESS_LLM_MULTIPLIER)),
            ],
        )
        .with_currency_prices(
            EUR,
            vec![
                platform_fee(&[(Monthly, dec!(269)), (Annual, dec!(2690))]),
                seats(&monthly_and_annual(dec!(14))),
                events(dec!(0.00027)),
                llm_tokens(Some(BUSINESS_LLM_MULTIPLIER)),
            ],
        )
        .with_feature(API_ACCESS, true)
        .with_feature(SSO, true)
        .with_cycle_limit(LLM_ALLOWANCE, dec!(100000000), 80)
        .with_pricing_page(
            "For companies that run on their data",
            false,
            vec![
                Text("1M events included, lower overage"),
                Feature(SSO),
                Feature(LLM_ALLOWANCE),
            ],
        ),
        plan(
            ENTERPRISE,
            4,
            vec![platform_fee(&[
                (Monthly, dec!(1500)),
                (Annual, dec!(15000)),
            ])],
        )
        .with_currency_prices(
            EUR,
            vec![platform_fee(&[
                (Monthly, dec!(1400)),
                (Annual, dec!(14000)),
            ])],
        )
        .with_feature(API_ACCESS, true)
        .with_feature(SSO, true)
        .with_pricing_page(
            "For large teams",
            false,
            vec![
                Feature(SSO),
                Feature(API_ACCESS),
                Text("Custom contracts and a dedicated success manager"),
            ],
        )
        .sold_by_sales("Talk to sales", CONTACT_SALES, true),
        // Off the ladder: sold to verified students, never offered on the pricing page.
        Plan::new(
            STUDENTS,
            EUR,
            PlanTypeEnum::Standard,
            vec![platform_fee(&[(Monthly, dec!(9))])],
        )
        .with_cadence_options(&[Monthly])
        .with_currency_prices(BRL, vec![platform_fee(&[(Monthly, dec!(49))])])
        .with_eligibility(Eligibility {
            countries: vec![],
            properties: vec![PropertyMatch {
                key: STUDENT_PROPERTY.to_string(),
                values: vec![VERIFIED.to_string()],
            }],
        }),
    ]
}

fn coupons(now: NaiveDate) -> Vec<CouponSpec> {
    let ago = |days: u64| now.checked_sub_days(Days::new(days)).unwrap_or(now);
    let students_from = ago(40);
    vec![
        CouponSpec {
            code: YC_CODE,
            name: "YC 2026",
            percent_off: dec!(50),
            plans: vec![PRO],
            // Twelve monthly invoices on Pro Monthly, the year's one on Pro Annual.
            months: Some(12),
            eligibility: None,
            valid_from: Some(ago(200)),
            expires_on: None,
            redemption_limit: None,
        },
        CouponSpec {
            code: STUDENTS_CODE,
            name: "Students in Brazil",
            percent_off: dec!(40),
            plans: vec![STARTER, PRO],
            months: Some(6),
            eligibility: Some(Eligibility {
                countries: vec!["BR".to_string()],
                properties: vec![PropertyMatch {
                    key: STUDENT_PROPERTY.to_string(),
                    values: vec![VERIFIED.to_string()],
                }],
            }),
            valid_from: Some(students_from),
            expires_on: students_from.checked_add_months(Months::new(6)),
            redemption_limit: Some(500),
        },
    ]
}

fn customers(now: NaiveDate) -> Vec<Customer> {
    let ago = |days: u64| now.checked_sub_days(Days::new(days)).unwrap_or(now);
    let mut student = customer(
        "Lucas Almeida",
        BRL,
        "BR",
        "São Paulo",
        vec![
            Subscription::active(STARTER, ago(25))
                .on_option(MONTHLY)
                .with_seats(2)
                .with_coupon(STUDENTS_CODE),
        ],
    );
    student.custom_properties = serde_json::json!({ STUDENT_PROPERTY: VERIFIED });

    vec![
        customer(
            "Bluegill Robotics",
            USD,
            "US",
            "Austin",
            vec![
                Subscription::active(STARTER, ago(152))
                    .on_option(MONTHLY)
                    .with_seats(5),
            ],
        ),
        customer(
            "Northbeam Retail",
            USD,
            "US",
            "New York",
            vec![
                Subscription::active(PRO, ago(221))
                    .on_option(ANNUAL)
                    .with_seats(14),
            ],
        ),
        customer(
            "Kranich Logistik GmbH",
            EUR,
            "DE",
            "Hamburg",
            vec![
                Subscription::active(PRO, ago(187))
                    .on_option(MONTHLY)
                    .with_seats(9),
            ],
        ),
        customer(
            "Maison Verlaine",
            EUR,
            "FR",
            "Lyon",
            vec![
                Subscription::active(BUSINESS, ago(298))
                    .on_option(ANNUAL)
                    .with_seats(40),
            ],
        ),
        customer(
            "Quillstack",
            USD,
            "US",
            "San Francisco",
            vec![
                Subscription::active(PRO, ago(75))
                    .on_option(ANNUAL)
                    .with_seats(6)
                    .with_coupon(YC_CODE),
            ],
        ),
        student,
    ]
}

pub fn currencies_scenario(now: NaiveDate) -> Scenario {
    let mut events_metric = metric(
        EVENTS_METRIC,
        "Tracked events",
        BillingMetricAggregateEnum::Sum,
        "value",
    );
    events_metric.unit_label = Some(UnitLabel {
        singular: "event",
        plural: "events",
    });
    let mut llm_metric = metric(
        LLM_METRIC,
        "LLM tokens",
        BillingMetricAggregateEnum::Sum,
        "tokens",
    );
    llm_metric.unit_label = Some(UnitLabel {
        singular: "token",
        plural: "tokens",
    });
    llm_metric.usage = Some(llm_usage());
    // Before the oldest subscription, so every period it bills is rated.
    let rates_from = now.checked_sub_days(Days::new(365)).unwrap_or(now);

    let mut acme = org("US-ACME-ANALYTICS", "Boston", "02110");
    acme.country = Some("US");
    acme.state = Some("MA");
    acme.tax_resolver = Some(TaxResolverEnum::Manual);

    Scenario {
        name: "Acme Analytics".to_string(),
        metrics: vec![events_metric, llm_metric],
        rate_cards: vec![llm_rate_card(rates_from)],
        features: features(),
        plans: plans(),
        customers: customers(now),
        organization: Some(acme),
        add_ons: vec![],
        quotes: vec![],
        credits: None,
        credit_units: vec![],
        promo_codes: vec![],
        coupons: coupons(now),
        customer_properties: vec![CustomerPropertySpec {
            key: STUDENT_PROPERTY,
            name: "Student",
            options: vec![VERIFIED, PENDING],
        }],
        reporting_currency: Some(USD.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::scenarios::domain::PlanEntitlementSpec;
    use meteroid_store::domain::eligibility::{CustomerFacts, check};

    fn scenario() -> Scenario {
        currencies_scenario(NaiveDate::from_ymd_opt(2026, 11, 23).unwrap())
    }

    /// The cadences a charge is priced on; usage has none of its own.
    fn cadences(component: &PriceComponent) -> Vec<BillingPeriodEnum> {
        match &component.fee {
            FeeType::Rate { rates } | FeeType::Slot { rates, .. } => {
                rates.iter().map(|r| r.term).collect()
            }
            _ => vec![],
        }
    }

    /// `plan`'s components in `currency`: its own in its currency, else those priced in it.
    fn in_currency<'a>(plan: &'a Plan, currency: &str) -> Option<&'a [PriceComponent]> {
        if plan.currency == currency {
            return Some(&plan.components);
        }
        plan.currency_prices
            .iter()
            .find(|p| p.currency == currency)
            .map(|p| p.components.as_slice())
    }

    /// Every charge of `plan` is priced in `currency` on `terms`; usage on no cadence of its own.
    fn assert_priced(plan: &Plan, currency: &str, terms: &[BillingPeriodEnum]) {
        let components = in_currency(plan, currency).expect(currency);
        for own in &plan.components {
            let priced = components
                .iter()
                .find(|c| c.name == own.name)
                .unwrap_or_else(|| panic!("{} of {} in {currency}", own.name, plan.name));
            let on = cadences(priced);
            assert!(
                on.is_empty() || on == terms,
                "{} of {} in {currency}",
                own.name,
                plan.name
            );
        }
    }

    fn facts(customer: &Customer) -> CustomerFacts {
        let country = customer
            .billing_address
            .as_ref()
            .and_then(|a| a.country.as_ref())
            .map(|c| c.code.clone());
        CustomerFacts::new(country, &customer.custom_properties)
    }

    fn coupon(code: &str) -> CouponSpec {
        scenario()
            .coupons
            .into_iter()
            .find(|p| p.code == code)
            .expect(code)
    }

    #[test]
    fn every_ladder_plan_sells_in_usd_and_eur_and_starter_and_pro_in_brl_monthly() {
        let scenario = scenario();
        for plan in scenario
            .plans
            .iter()
            .filter(|p| p.self_service_rank.is_some())
        {
            assert_eq!(plan.currency, USD);
            for currency in [USD, EUR] {
                assert_priced(plan, currency, &[Monthly, Annual]);
            }
            if [BUSINESS, ENTERPRISE].contains(&plan.name.as_str()) {
                assert!(in_currency(plan, BRL).is_none());
            } else {
                assert_priced(plan, BRL, &[Monthly]);
            }
        }
    }

    fn plan(name: &str) -> Plan {
        scenario()
            .plans
            .into_iter()
            .find(|p| p.name == name)
            .expect(name)
    }

    /// Off the ladder, in EUR and BRL monthly, for verified students only.
    #[test]
    fn students_is_off_the_ladder_for_verified_students() {
        let students = plan(STUDENTS);
        assert_eq!(students.self_service_rank, None);
        assert_eq!(students.pricing_page, None);
        assert_eq!(students.currency, EUR);
        assert_priced(&students, EUR, &[Monthly]);
        assert_priced(&students, BRL, &[Monthly]);
        assert!(in_currency(&students, USD).is_none());
        let rule = students.eligibility.expect("a rule");
        for customer in &scenario().customers {
            assert_eq!(
                check(&rule, &facts(customer)).is_ok(),
                customer.name == "Lucas Almeida",
                "{}",
                customer.name
            );
        }
    }

    #[test]
    fn enterprise_is_sold_by_sales_its_price_hidden() {
        let enterprise = plan(ENTERPRISE);
        assert_eq!(enterprise.self_service_rank, Some(4));
        let page = enterprise.pricing_page.expect("its copy");
        assert_eq!(page.tagline, "For large teams");
        assert_eq!(page.cta_label, Some("Talk to sales"));
        assert!(page.sales_led && page.hide_price);
        assert_eq!(page.contact_url, Some(CONTACT_SALES));
    }

    /// The card has a rate in each currency for every key; Pro pays them, Business 20% off.
    #[test]
    fn pro_and_business_price_llm_tokens_by_the_rate_card() {
        let scenario = scenario();
        let [card] = scenario.rate_cards.as_slice() else {
            panic!("one rate card");
        };
        assert_eq!(card.metric_code, LLM_METRIC);
        assert_eq!(card.unit_size, dec!(1000000));
        assert!(card.rates.iter().any(|r| r.key == vec!["*", "*"]));
        for row in &card.rates {
            let currencies: Vec<&str> = row.rates.iter().map(|(c, _)| *c).collect();
            assert_eq!(currencies, vec![USD, EUR, BRL], "{:?}", row.key);
        }
        for (name, multiplier, currencies) in [
            (PRO, None, vec![USD, EUR, BRL]),
            (BUSINESS, Some(dec!(0.8)), vec![USD, EUR]),
        ] {
            let plan = plan(name);
            for currency in currencies {
                let llm = in_currency(&plan, currency)
                    .and_then(|components| components.iter().find(|c| c.name == LLM_TOKENS))
                    .unwrap_or_else(|| panic!("{name} in {currency}"));
                assert!(matches!(
                    &llm.fee,
                    FeeType::RateCard { rate_card, multiplier: m, .. }
                        if rate_card == LLM_CARD && *m == multiplier
                ));
            }
        }
    }

    /// A feature a card highlights is on a product the plan sells, and the plan has it.
    #[test]
    fn every_highlighted_feature_is_one_the_plan_has() {
        let scenario = scenario();
        for plan in &scenario.plans {
            let highlights = plan.pricing_page.iter().flat_map(|page| &page.highlights);
            for line in highlights {
                let Feature(code) = line else {
                    continue;
                };
                let feature = scenario
                    .features
                    .iter()
                    .find(|f| f.code == *code)
                    .expect(code);
                let product = feature.product.expect("on a product");
                assert!(
                    plan.components.iter().any(|c| c.name == product),
                    "{} sells {product}",
                    plan.name
                );
                let granted = match &feature.kind {
                    FeatureKind::Boolean { default_enabled } => {
                        *default_enabled
                            || plan.entitlements.iter().any(|e| {
                                *e == PlanEntitlementSpec::Boolean {
                                    feature_code: code.to_string(),
                                    enabled: true,
                                }
                            })
                    }
                    FeatureKind::Metered { .. } => plan.entitlement(code).is_some(),
                    FeatureKind::Number { .. } => true,
                };
                assert!(granted, "{} has {code}", plan.name);
            }
        }
    }

    #[test]
    fn customers_of_every_currency_subscribe_on_an_option() {
        let scenario = scenario();
        for currency in [USD, EUR, BRL] {
            assert!(scenario.customers.iter().any(|c| c.currency == currency));
        }
        for customer in &scenario.customers {
            for sub in &customer.subscriptions {
                assert!(sub.option.is_some(), "{}", customer.name);
                let plan = scenario
                    .plans
                    .iter()
                    .find(|p| p.name == sub.plan_name)
                    .unwrap();
                assert!(in_currency(plan, &customer.currency).is_some());
            }
        }
    }

    #[test]
    fn the_ladder_has_four_ranks_and_one_popular_plan() {
        let scenario = scenario();
        let ranks: Vec<Option<i32>> = scenario.plans.iter().map(|p| p.self_service_rank).collect();
        assert_eq!(ranks, vec![Some(1), Some(2), Some(3), Some(4), None]);
        let popular: Vec<&str> = scenario
            .plans
            .iter()
            .filter(|p| p.pricing_page.as_ref().is_some_and(|page| page.popular))
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(popular, vec![PRO]);
        assert!(
            scenario
                .plans
                .iter()
                .filter(|p| p.self_service_rank.is_some())
                .all(|p| {
                    p.pricing_page
                        .as_ref()
                        .is_some_and(|page| page.highlights.len() == 3)
                })
        );
    }

    #[test]
    fn yc_2026_is_half_off_pro_for_12_months_for_whoever_has_its_code() {
        let yc = coupon(YC_CODE);
        assert_eq!(yc.name, "YC 2026");
        assert_eq!(yc.percent_off, dec!(50));
        assert_eq!(yc.plans, vec![PRO]);
        assert_eq!(yc.months, Some(12));
        assert_eq!(yc.eligibility, None);
    }

    /// Lucas's `student` is a defined customer property, one of its options.
    #[test]
    fn the_student_property_is_defined_with_its_options() {
        let scenario = scenario();
        let student = scenario
            .customer_properties
            .iter()
            .find(|p| p.key == STUDENT_PROPERTY)
            .expect("student");
        assert!(student.options.contains(&VERIFIED));
    }

    #[test]
    fn students_in_brazil_is_for_verified_students_in_brazil_for_six_months() {
        let students = coupon(STUDENTS_CODE);
        assert_eq!(students.percent_off, dec!(40));
        assert_eq!(students.plans, vec![STARTER, PRO]);
        assert_eq!(students.redemption_limit, Some(500));
        assert_eq!(students.months, Some(6));
        let from = students.valid_from.unwrap();
        assert_eq!(students.expires_on, from.checked_add_months(Months::new(6)));
        let rule = students.eligibility.unwrap();
        for customer in &scenario().customers {
            assert_eq!(
                check(&rule, &facts(customer)).is_ok(),
                customer.currency == BRL,
                "{}",
                customer.name
            );
        }
        let lucas = scenario()
            .customers
            .into_iter()
            .find(|c| c.name == "Lucas Almeida");
        let mut unverified = facts(&lucas.expect("Lucas Almeida"));
        unverified.properties.clear();
        assert!(check(&rule, &unverified).is_err());
    }

    #[test]
    fn each_coupon_is_applied_once() {
        let scenario = scenario();
        let applied: Vec<(&str, &str, Option<&str>)> = scenario
            .customers
            .iter()
            .flat_map(|c| &c.subscriptions)
            .flat_map(|s| {
                s.coupons
                    .iter()
                    .map(move |code| (*code, s.plan_name.as_str(), s.option))
            })
            .collect();
        assert_eq!(
            applied,
            vec![
                (YC_CODE, PRO, Some(ANNUAL)),
                (STUDENTS_CODE, STARTER, Some(MONTHLY))
            ]
        );
    }
}
