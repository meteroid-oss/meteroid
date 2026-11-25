//! A usage price rated by a rate card: each part of a period rated by the entries in force on
//! its days in the price's currency, every rate per the card's unit size and scaled by the
//! price's multiplier, and usage no entry rates holding the invoice until a rate is added (R-132).
use std::collections::HashMap;
use std::sync::Arc;

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, RateCardId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::price_components::UsagePricingModel;
use meteroid_store::domain::products::ProductNew;
use meteroid_store::domain::rate_cards::{
    RateCardDenomination, RateCardNew, RateCardRateInput, RateCardRatesSet, RateCardUnmatched,
};
use meteroid_store::domain::{Invoice, UsagePeriod};
use meteroid_store::repositories::RateCardsInterface;
use meteroid_store::repositories::products::ProductInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, m, d).unwrap()
}

fn at(m: u32, d: u32) -> chrono::DateTime<chrono::Utc> {
    day(m, d).and_hms_opt(9, 0, 0).unwrap().and_utc()
}

fn set_usage(
    usage: &MutableMockUsageClient,
    from: NaiveDate,
    to: NaiveDate,
    units: &[(&str, i64)],
) {
    let (start, end) = (
        from.and_time(chrono::NaiveTime::MIN),
        to.and_time(chrono::NaiveTime::MIN),
    );
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: units
                .iter()
                .map(|(model, units)| GroupedUsageData {
                    value: Decimal::from(*units),
                    dimensions: HashMap::from([("model".to_string(), (*model).to_string())]),
                })
                .collect(),
            period: UsagePeriod { start, end },
        },
    );
}

fn rate_in(model: &str, currency: &str, rate: Decimal) -> RateCardRateInput {
    RateCardRateInput {
        key: vec![model.to_string()],
        currency: Some(currency.to_string()),
        rate,
        label: None,
    }
}

fn rate(model: &str, rate: Decimal) -> RateCardRateInput {
    rate_in(model, "EUR", rate)
}

/// Sets `rates` from `from`, written on `today`.
async fn set_rates(
    env: &TestEnv,
    card: RateCardId,
    from: NaiveDate,
    rates: Vec<RateCardRateInput>,
    today: NaiveDate,
) {
    env.store()
        .set_rate_card_rates(
            Actor::System,
            TENANT_ID,
            card,
            RateCardRatesSet {
                effective_from: Some(from),
                rates,
                remove: Vec::new(),
            },
            today,
        )
        .await
        .expect("the rates");
}

async fn rate_card(
    env: &TestEnv,
    code: &str,
    unit_size: Decimal,
    on_unmatched: RateCardUnmatched,
) -> RateCardId {
    env.store()
        .create_rate_card(
            Actor::System,
            RateCardNew {
                tenant_id: TENANT_ID,
                code: code.to_string(),
                name: "Models".to_string(),
                metric_id: METRIC_BANDWIDTH,
                dimensions: vec!["model".to_string()],
                denomination: RateCardDenomination::Money,
                unit_size,
                on_unmatched,
            },
        )
        .await
        .expect("a card")
        .id
}

/// A card by model on bandwidth, a/* at 0.10 and b at 0.20 from Jan 1, and a monthly plan
/// billing its usage at half its rates.
async fn rated_plan(env: &TestEnv, on_unmatched: RateCardUnmatched) -> (RateCardId, PlanVersionId) {
    let card = rate_card(env, "models", Decimal::ONE, on_unmatched).await;
    set_rates(
        env,
        card,
        day(1, 1),
        vec![rate("a/*", dec!(0.10)), rate("b", dec!(0.20))],
        day(1, 1),
    )
    .await;
    let version = card_plan(env, card, "EUR").await;
    (card, version)
}

/// A monthly plan in `currency` billing the card's usage at half its rates.
async fn card_plan(env: &TestEnv, card: RateCardId, currency: &'static str) -> PlanVersionId {
    let product = env
        .store()
        .create_product(
            Actor::System,
            ProductNew {
                name: "Model tokens".to_string(),
                description: None,
                tenant_id: TENANT_ID,
                family_id: PRODUCT_FAMILY_ID,
                catalog: true,
                tax_category_id: None,
                icon: None,
                color: None,
            },
        )
        .await
        .expect("a product")
        .id;
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Rated", version)
        .currency(currency)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Tokens",
            product,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::RateCard {
                rate_card_id: card,
                multiplier: Some(dec!(0.5)),
            },
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("plan");
    version
}

fn describe(invoice: &Invoice) -> Vec<String> {
    let mut lines = Vec::new();
    for line in &invoice.line_items {
        lines.push(format!(
            "{} {}..{} {}",
            line.name, line.start_date, line.end_date, line.amount_subtotal
        ));
        for sub_line in &line.sub_lines {
            lines.push(format!(
                "  {} {} x {} = {}",
                sub_line.name, sub_line.quantity, sub_line.unit_price, sub_line.total
            ));
        }
    }
    lines
}

/// b goes from 0.20 to 0.30 on Jan 15: January's usage is cut there, each part rated at half
/// its entries' rates.
/// Rules: R-132
#[tokio::test]
async fn rate_card_usage_bills_each_part_at_its_entries() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let (card, version) = rated_plan(&env, RateCardUnmatched::Hold).await;
    set_usage(&usage, day(1, 1), day(1, 15), &[("a/x", 100), ("b", 10)]);
    set_usage(&usage, day(1, 15), day(2, 1), &[("a/x", 100), ("b", 10)]);

    env.set_now(at(1, 1));
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(at(1, 10));
    set_rates(
        &env,
        card,
        day(1, 15),
        vec![rate("b", dec!(0.30))],
        day(1, 10),
    )
    .await;
    env.run_until(day(2, 2)).await;

    let lines: Vec<String> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(describe)
        .collect();
    assert_eq!(
        lines,
        [
            "Tokens 2025-01-01..2025-01-15 600",
            "  a/x 100 x 0.050 = 500",
            "  b 10 x 0.100 = 100",
            "Tokens 2025-01-15..2025-02-01 650",
            "  a/x 100 x 0.050 = 500",
            "  b 10 x 0.150 = 150",
        ]
    );
}

/// A model no entry rates holds the invoice; its first rate, dated back to the period's start,
/// releases it.
/// Rules: R-132, R-131
#[tokio::test]
async fn an_unrated_model_holds_the_invoice_until_it_is_rated() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let (card, version) = rated_plan(&env, RateCardUnmatched::Hold).await;
    set_usage(&usage, day(1, 1), day(2, 1), &[("a/x", 100), ("c", 10)]);

    env.set_now(at(1, 1));
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_until(day(2, 3)).await;
    assert!(
        env.get_invoices(sub_id).await.is_empty(),
        "an unrated model holds the invoice"
    );

    set_rates(&env, card, day(1, 1), vec![rate("c", dec!(1))], day(2, 3)).await;
    env.run_until(day(2, 6)).await;
    let lines: Vec<String> = env
        .get_invoices(sub_id)
        .await
        .iter()
        .flat_map(describe)
        .collect();
    assert_eq!(
        lines,
        [
            "Tokens 2025-01-01..2025-02-01 1000",
            "  a/x 100 x 0.050 = 500",
            "  c 10 x 0.5 = 500",
        ]
    );
}

fn totals(invoice: &Invoice) -> Vec<String> {
    invoice
        .line_items
        .iter()
        .flat_map(|line| {
            std::iter::once(format!("{} {}", line.name, line.amount_subtotal)).chain(
                line.sub_lines
                    .iter()
                    .map(|sub_line| format!("  {} {}", sub_line.name, sub_line.total)),
            )
        })
        .collect()
}

/// One card per 1M tokens, rates in EUR and USD: a EUR and a USD subscription read the same
/// usage, each rated in its own currency at half the card's rates. 2M of a/x and 0.5M of b bill
/// 1.50 × 2 + 5.00 × 0.5 = 5.50 in EUR and 1.75 × 2 + 6.00 × 0.5 = 6.50 in USD.
/// Rules: R-132
#[tokio::test]
async fn a_card_rates_each_price_in_its_currency_per_its_unit_size() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let card = rate_card(&env, "per-1m", dec!(1000000), RateCardUnmatched::Hold).await;
    set_rates(
        &env,
        card,
        day(1, 1),
        vec![
            rate_in("a/*", "EUR", dec!(3)),
            rate_in("a/*", "USD", dec!(3.50)),
            rate_in("b", "EUR", dec!(10)),
            rate_in("b", "USD", dec!(12)),
        ],
        day(1, 1),
    )
    .await;
    let eur = card_plan(&env, card, "EUR").await;
    let usd = card_plan(&env, card, "USD").await;
    set_usage(
        &usage,
        day(1, 1),
        day(2, 1),
        &[("a/x", 2_000_000), ("b", 500_000)],
    );

    env.set_now(at(1, 1));
    // Each its own customer: a held subscription holds its customer's billing.
    let mut subscriptions = Vec::new();
    for (version, customer) in [(eur, CUST_UBER_ID), (usd, CUST_SPOTIFY_ID)] {
        subscriptions.push(
            subscription()
                .customer(customer)
                .plan_version(version)
                .start_date(day(1, 1))
                .on_start()
                .no_trial()
                .create(env.services())
                .await,
        );
    }
    env.run_until(day(2, 2)).await;

    let billed = |sub_id| {
        let env = &env;
        async move {
            let invoices = env.get_invoices(sub_id).await;
            let renewal = invoices
                .iter()
                .find(|invoice| !invoice.line_items.is_empty())
                .expect("January's usage invoiced");
            (renewal.currency.clone(), totals(renewal))
        }
    };
    assert_eq!(
        billed(subscriptions[0]).await,
        (
            "EUR".to_string(),
            vec![
                "Tokens 550".to_string(),
                "  a/x 300".to_string(),
                "  b 250".to_string()
            ]
        )
    );
    assert_eq!(
        billed(subscriptions[1]).await,
        (
            "USD".to_string(),
            vec![
                "Tokens 650".to_string(),
                "  a/x 350".to_string(),
                "  b 300".to_string()
            ]
        )
    );
}

/// b has a rate in EUR only: the USD subscription's usage of b is unmatched and holds its
/// invoice while the EUR one bills; b's first USD rate, dated back to Jan 1, releases it.
/// Rules: R-132, R-131
#[tokio::test]
async fn a_key_without_a_rate_in_the_prices_currency_is_unmatched() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    let card = rate_card(&env, "eur-only-b", Decimal::ONE, RateCardUnmatched::Hold).await;
    set_rates(
        &env,
        card,
        day(1, 1),
        vec![
            rate_in("a/*", "EUR", dec!(0.10)),
            rate_in("a/*", "USD", dec!(0.12)),
            rate_in("b", "EUR", dec!(0.20)),
        ],
        day(1, 1),
    )
    .await;
    let eur = card_plan(&env, card, "EUR").await;
    let usd = card_plan(&env, card, "USD").await;
    set_usage(&usage, day(1, 1), day(2, 1), &[("a/x", 100), ("b", 10)]);

    env.set_now(at(1, 1));
    // Each its own customer: a held subscription holds its customer's billing.
    let mut subscriptions = Vec::new();
    for (version, customer) in [(eur, CUST_UBER_ID), (usd, CUST_SPOTIFY_ID)] {
        subscriptions.push(
            subscription()
                .customer(customer)
                .plan_version(version)
                .start_date(day(1, 1))
                .on_start()
                .no_trial()
                .create(env.services())
                .await,
        );
    }
    env.run_until(day(2, 3)).await;
    let lines = |sub_id| {
        let env = &env;
        async move {
            env.get_invoices(sub_id)
                .await
                .iter()
                .flat_map(totals)
                .collect::<Vec<String>>()
        }
    };
    assert_eq!(
        lines(subscriptions[0]).await,
        ["Tokens 600", "  a/x 500", "  b 100"],
        "EUR: 100 × 0.05 + 10 × 0.10"
    );
    assert!(
        lines(subscriptions[1]).await.is_empty(),
        "USD: b has no rate in USD, its invoice is held"
    );

    set_rates(
        &env,
        card,
        day(1, 1),
        vec![rate_in("b", "USD", dec!(0.30))],
        day(2, 3),
    )
    .await;
    env.run_until(day(2, 6)).await;
    assert_eq!(
        lines(subscriptions[1]).await,
        ["Tokens 750", "  a/x 600", "  b 150"],
        "USD: 100 × 0.06 + 10 × 0.15"
    );
}

/// A price no longer knows its metric: the charge it is linked to does. A card on bandwidth
/// prices a charge billing bandwidth, and is refused on a charge billing the database size, both
/// when the charge is created with it and when it is set in a cell.
/// Rules: R-139, R-132
#[tokio::test]
async fn a_rate_card_price_is_refused_on_a_charge_of_another_metric() {
    use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
    use meteroid_store::domain::price_components::{PriceEntry, PriceInput, ProductRef};
    use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
    use meteroid_store::repositories::price_components::PriceComponentInterface;

    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MutableMockUsageClient::new()))
            .await;
    let card = rate_card(
        &env,
        "bandwidth-models",
        Decimal::ONE,
        RateCardUnmatched::Hold,
    )
    .await;
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Draft", PlanVersionId::new())
        .draft(version, 2)
        .seed(&mut env.conn().await)
        .await
        .expect("a plan with a draft");
    let by_card = || PriceInput {
        cadence: BillingPeriodEnum::Monthly,
        currency: "EUR".to_string(),
        pricing: Pricing::Usage(UsagePricingModel::RateCard {
            rate_card_id: card,
            multiplier: None,
        }),
        credit_unit: None,
    };
    let usage_of = |name: &str, metric_id| ProductRef::New {
        name: name.to_string(),
        fee_type: FeeTypeEnum::Usage,
        fee_structure: FeeStructure::Usage {
            metric_id,
            model: UsageModel::RateCard,
        },
    };

    let refused = env
        .store()
        .create_charge_from_ref(
            "Storage".to_string(),
            usage_of("Storage", METRIC_DATABASE_SIZE),
            None,
            vec![PriceEntry::New(by_card())],
            version,
            TENANT_ID,
        )
        .await;
    assert!(
        refused.is_err_and(|err| format!("{err:?}").contains("rates another metric")),
        "a bandwidth card on a database-size charge"
    );

    env.store()
        .create_charge_from_ref(
            "Bandwidth".to_string(),
            usage_of("Bandwidth", METRIC_BANDWIDTH),
            None,
            vec![PriceEntry::New(by_card())],
            version,
            TENANT_ID,
        )
        .await
        .expect("a bandwidth card on a bandwidth charge");

    let storage = env
        .store()
        .create_charge_from_ref(
            "Storage".to_string(),
            usage_of("Storage", METRIC_DATABASE_SIZE),
            None,
            vec![PriceEntry::New(PriceInput {
                pricing: Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
                ..by_card()
            })],
            version,
            TENANT_ID,
        )
        .await
        .expect("a database-size charge at a unit rate");
    let refused = env
        .store()
        .set_component_price(TENANT_ID, storage.id, vec![], by_card())
        .await;
    assert!(
        refused.is_err_and(|err| format!("{err:?}").contains("rates another metric")),
        "a bandwidth card set in a database-size charge's cell"
    );
}
