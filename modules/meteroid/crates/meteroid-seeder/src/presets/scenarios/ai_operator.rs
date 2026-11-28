//! **Fjordline Inference**, an EU inference cloud reselling open and third-party models through
//! the LiteLLM gateway it runs, per million tokens: the "AI operator" template. Its usage follows
//! the LLM usage convention (`llm_tokens`, a sum of `tokens` by `model` and `token_class`), its
//! prices are a rate card, and a cost card holds what each model costs it at list price, which
//! the AI module's margin reads. Scale buys at 20% off, which loses money on GPT-4.1 output.

use super::common::{customer, metric, org};
use super::domain::{
    Customer, EventProperty, FeeType, Plan, PriceComponent, RateCardSpec, RateSpec, Scenario,
    Subscription, UnitLabel, UsageSample,
};
use chrono::{Days, NaiveDate};
use meteroid_store::domain::rate_cards::RateCardPurpose;
use meteroid_store::domain::{BillingMetricAggregateEnum, BillingPeriodEnum, PlanTypeEnum};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

const EUR: &str = "EUR";
pub const LLM_METRIC: &str = "llm_tokens";
pub const PRICE_CARD: &str = "llm-prices";
pub const COST_CARD: &str = "llm-costs";
const MODEL: &str = "model";
const TOKEN_CLASS: &str = "token_class";
const PER_MILLION: Decimal = dec!(1000000);
pub const PAY_AS_YOU_GO: &str = "Pay as you go";
pub const SCALE: &str = "Scale";

/// Calls by model and token class, of 1k to 40k tokens each. `qwen-2.5-72b` is in neither card's
/// named rates: its price is the `*` row's, and its cost is unknown until it is priced.
fn llm_usage() -> UsageSample {
    UsageSample {
        min: 1_000,
        max: 40_000,
        properties: vec![
            EventProperty {
                key: MODEL,
                values: vec![
                    ("mistral-large", 30),
                    ("llama-3.3-70b", 30),
                    ("gpt-4.1", 25),
                    ("claude-sonnet-4", 10),
                    ("qwen-2.5-72b", 5),
                ],
            },
            EventProperty {
                key: TOKEN_CLASS,
                values: vec![("input", 65), ("output", 25), ("cache_read", 10)],
            },
            EventProperty {
                key: "provider",
                values: vec![("fjordline", 1)],
            },
        ],
    }
}

fn rate(model: &'static str, class: &'static str, rate: Decimal) -> RateSpec {
    RateSpec {
        key: vec![model, class],
        label: None,
        rates: vec![(EUR, rate)],
    }
}

fn card(
    code: &'static str,
    name: &'static str,
    purpose: RateCardPurpose,
    effective_from: NaiveDate,
    rates: Vec<RateSpec>,
) -> RateCardSpec {
    RateCardSpec {
        code,
        name,
        metric_code: LLM_METRIC,
        dimensions: vec![MODEL, TOKEN_CLASS],
        unit_size: PER_MILLION,
        effective_from,
        rates,
        purpose,
    }
}

/// What Fjordline charges per 1M tokens, in EUR.
pub fn price_card(effective_from: NaiveDate) -> RateCardSpec {
    card(
        PRICE_CARD,
        "LLM prices",
        RateCardPurpose::Price,
        effective_from,
        vec![
            rate("mistral-large", "input", dec!(2.40)),
            rate("mistral-large", "output", dec!(7.20)),
            rate("llama-3.3-70b", "input", dec!(0.90)),
            rate("llama-3.3-70b", "output", dec!(1.20)),
            rate("gpt-4.1", "input", dec!(2.20)),
            rate("gpt-4.1", "output", dec!(8.40)),
            rate("claude-sonnet-4", "input", dec!(3.60)),
            rate("claude-sonnet-4", "output", dec!(18)),
            rate("*", "cache_read", dec!(0.30)),
            rate("*", "*", dec!(4)),
        ],
    )
}

/// What each model costs Fjordline per 1M tokens, in EUR: list prices for the third-party
/// models, GPU cost for the ones it hosts. A small seed, to be replaced by the operator's own.
pub fn cost_card(effective_from: NaiveDate) -> RateCardSpec {
    card(
        COST_CARD,
        "LLM costs",
        RateCardPurpose::Cost,
        effective_from,
        vec![
            rate("mistral-large", "input", dec!(1.80)),
            rate("mistral-large", "output", dec!(5.40)),
            rate("mistral-large", "cache_read", dec!(0.18)),
            rate("llama-3.3-70b", "*", dec!(0.35)),
            rate("gpt-4.1", "input", dec!(1.85)),
            rate("gpt-4.1", "output", dec!(7.40)),
            rate("gpt-4.1", "cache_read", dec!(0.46)),
            rate("claude-sonnet-4", "input", dec!(2.75)),
            rate("claude-sonnet-4", "output", dec!(13.80)),
            rate("claude-sonnet-4", "cache_read", dec!(0.28)),
        ],
    )
}

fn tokens(multiplier: Option<Decimal>) -> PriceComponent {
    PriceComponent {
        name: "Model tokens".to_string(),
        fee: FeeType::RateCard {
            metric_code: LLM_METRIC.to_string(),
            rate_card: PRICE_CARD.to_string(),
            multiplier,
            cadence: BillingPeriodEnum::Monthly,
        },
    }
}

fn plans() -> Vec<Plan> {
    vec![
        Plan::new(
            PAY_AS_YOU_GO,
            EUR,
            PlanTypeEnum::Standard,
            vec![tokens(None)],
        )
        .with_self_service_rank(1),
        Plan::new(
            SCALE,
            EUR,
            PlanTypeEnum::Standard,
            vec![tokens(Some(dec!(0.8)))],
        )
        .with_self_service_rank(2),
    ]
}

fn customers(now: NaiveDate) -> Vec<Customer> {
    let ago = |days: u64| now.checked_sub_days(Days::new(days)).unwrap_or(now);
    vec![
        customer(
            "Brightwater Logistics",
            EUR,
            "NL",
            "Rotterdam",
            vec![Subscription::active(SCALE, ago(120))],
        ),
        customer(
            "Okapi Health",
            EUR,
            "DE",
            "Berlin",
            vec![Subscription::active(PAY_AS_YOU_GO, ago(95))],
        ),
        customer(
            "Tidewell Energy",
            EUR,
            "NO",
            "Bergen",
            vec![Subscription::active(PAY_AS_YOU_GO, ago(60))],
        ),
        customer(
            "Ostrander Legal",
            EUR,
            "FR",
            "Paris",
            vec![Subscription::active(SCALE, ago(40))],
        ),
    ]
}

pub fn ai_operator_scenario(now: NaiveDate) -> Scenario {
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
    // Before the oldest subscription, so every period it bills is rated and costed.
    let rates_from = now.checked_sub_days(Days::new(180)).unwrap_or(now);

    let mut fjordline = org("NO123456789MVA", "Oslo", "0150");
    fjordline.country = Some("NO");

    Scenario {
        name: "Fjordline Inference".to_string(),
        metrics: vec![llm_metric],
        rate_cards: vec![price_card(rates_from), cost_card(rates_from)],
        features: vec![],
        plans: plans(),
        customers: customers(now),
        organization: Some(fjordline),
        add_ons: vec![],
        quotes: vec![],
        credits: None,
        credit_units: vec![],
        promo_codes: vec![],
        coupons: vec![],
        customer_properties: vec![],
        reporting_currency: Some(EUR.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        ai_operator_scenario(NaiveDate::from_ymd_opt(2026, 11, 26).unwrap())
    }

    #[test]
    fn one_price_card_and_one_cost_card_rate_the_llm_metric() {
        let scenario = scenario();
        let purposes: Vec<(&str, RateCardPurpose)> = scenario
            .rate_cards
            .iter()
            .map(|card| (card.code, card.purpose))
            .collect();
        assert_eq!(
            purposes,
            vec![
                (PRICE_CARD, RateCardPurpose::Price),
                (COST_CARD, RateCardPurpose::Cost)
            ]
        );
        assert!(
            scenario
                .rate_cards
                .iter()
                .all(|card| card.metric_code == LLM_METRIC
                    && card.dimensions == vec![MODEL, TOKEN_CLASS])
        );
    }

    #[test]
    fn plans_price_with_the_price_card_never_the_cost_card() {
        for plan in scenario().plans {
            for component in plan.components {
                match component.fee {
                    FeeType::RateCard { rate_card, .. } => assert_eq!(rate_card, PRICE_CARD),
                    _ => panic!("{} prices tokens by the price card", plan.name),
                }
            }
        }
    }

    /// Scale's 20% off makes GPT-4.1 output cost more than it is sold for.
    #[test]
    fn scale_loses_money_on_gpt_4_1_output() {
        let rate_of = |card: &RateCardSpec| {
            card.rates
                .iter()
                .find(|r| r.key == vec!["gpt-4.1", "output"])
                .map(|r| r.rates[0].1)
                .unwrap()
        };
        let now = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        let price = rate_of(&price_card(now)) * dec!(0.8);
        let cost = rate_of(&cost_card(now));
        assert!(price < cost, "{price} < {cost}");
    }
}
