//! Usage is paid only by credit effective before its line's usage ends: an
//! immediate cancellation dates its last line by the cancellation instant, a plan pack pays from
//! its period's start, and credit taking effect after an immediate plan change pays only the new
//! plan's usage.
use chrono::{NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use common_domain::actor::Actor;
use common_domain::ids::{CreditGrantId, CustomerId, InvoiceId, SubscriptionId};
use diesel_models::customer_credit::{CustomerCreditEntryRow, CustomerCreditGrantRow};
use meteroid_store::clients::usage::{GroupedUsageData, UsageData};
use meteroid_store::domain::credit::GrantClassification;
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceStatusEnum};
use meteroid_store::domain::price_components::{CreditExpiry, ProductRef, UsagePricingModel};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::{Invoice, UsagePeriod};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::services::SellCustomerCredit;

use super::credit_plan_change::{
    LARGE, add_component, change_on, date, grant_rows, midnight, money, new_pack_product, pay,
    set_usage, start, usage_env,
};
use super::usage::{PLAN_VERSION_USAGE_ONLY_ID, seed_usage_only_plan};
use crate::data::ids::*;
use crate::harness::TestEnv;

/// Large bills bandwidth at EUR 0.10 a unit in arrears, after `pack` if given.
async fn bandwidth_plan(env: &TestEnv, pack: Option<Pricing>) {
    if let Some(pack) = pack {
        add_component(env, "Credit", LARGE, new_pack_product("Credit"), pack).await;
    }
    add_component(
        env,
        "Bandwidth",
        LARGE,
        ProductRef::New {
            name: "Bandwidth".to_owned(),
            fee_type: FeeTypeEnum::Usage,
            fee_structure: FeeStructure::Usage {
                metric_id: METRIC_BANDWIDTH,
                model: UsageModel::PerUnit,
            },
        },
        Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
    )
    .await;
}

/// 300 units (EUR 30.00) in every non-empty usage window, whatever its bounds.
fn bandwidth_everywhere(usage: &meteroid_store::clients::usage::MutableMockUsageClient) {
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(300),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: date(2024, 1, 1).and_time(NaiveTime::MIN),
                end: date(2024, 1, 1).and_time(NaiveTime::MIN),
            },
        },
    );
}

async fn movements(env: &TestEnv, grant: CreditGrantId) -> Vec<String> {
    let mut conn = env.conn().await;
    CustomerCreditEntryRow::list_by_grant(&mut conn, TENANT_ID, grant)
        .await
        .expect("movements")
        .into_iter()
        .map(|entry| format!("{} {}", entry.kind, entry.amount))
        .collect()
}

/// The grant of `classification` an invoice funded.
async fn grant_of(
    env: &TestEnv,
    customer: CustomerId,
    invoice: InvoiceId,
    classification: &str,
) -> CustomerCreditGrantRow {
    grant_rows(env, customer)
        .await
        .into_iter()
        .find(|grant| grant.invoice_id == Some(invoice) && grant.classification == classification)
        .unwrap_or_else(|| panic!("no {classification} grant of invoice {invoice}"))
}

/// The invoice whose usage lines start on `start` and end on `end`, issued.
async fn usage_invoice(
    env: &TestEnv,
    subscription: SubscriptionId,
    start: NaiveDate,
    end: NaiveDate,
) -> Invoice {
    let invoice = env
        .get_invoices(subscription)
        .await
        .into_iter()
        .find(|invoice| {
            invoice.line_items.iter().any(|line| {
                line.metric_id.is_some() && line.start_date == start && line.end_date == end
            })
        })
        .unwrap_or_else(|| panic!("no invoice billing usage [{start}, {end})"));
    if invoice.status != InvoiceStatusEnum::Draft {
        return invoice;
    }
    env.services()
        .finalize_invoice(Actor::System, invoice.id, TENANT_ID)
        .await
        .expect("issued")
        .invoice
}

async fn cancel_now(env: &TestEnv, subscription: SubscriptionId) {
    env.services()
        .cancel_subscription(
            Actor::System,
            subscription,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancelled");
    env.process_due_events().await;
}

/// EUR 50.00 of goodwill credit added by hand now, at the wall clock: effective from now on.
async fn goodwill_now(env: &TestEnv) -> CreditGrantId {
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("wallet");
    env.store()
        .fund_customer_credit_wallet(FundCustomerCreditWallet {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            wallet_id: wallet.id,
            amount_minor: 5_000,
            classification: GrantClassification::Promotional,
            priority: DEFAULT_PRIORITY,
            expires_at: None,
            idempotency_key: uuid::Uuid::now_v7(),
            reason: "Goodwill".to_owned(),
            reference: None,
            actor_id: None,
            cost: None,
        })
        .await
        .expect("goodwill added")
        .grant_id
}

/// M1. Ten days into its period, credit added this morning, then an immediate cancellation: the
/// last line's usage, read up to the cancellation, is dated by it, so that credit (effective
/// after midnight) pays its EUR 30.00 before tax.
#[tokio::test]
async fn an_immediate_cancellation_mid_period_draws_credit_added_that_day() {
    let (env, usage) = usage_env().await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, -1).await;
    bandwidth_everywhere(&usage);
    let today = Utc::now().date_naive();
    let started = today - chrono::Days::new(10);
    let sub = start(&env, CUST_UBER_ID, PLAN_VERSION_USAGE_ONLY_ID, started).await;
    let goodwill = goodwill_now(&env).await;

    cancel_now(&env, sub).await;
    let last = usage_invoice(&env, sub, started, today).await;
    assert_eq!(money(&last), (3_000, 3_000, 0, 0, 0, 0));
    assert_eq!(
        movements(&env, goodwill).await,
        vec!["FUND 5000", "DRAW -3000"]
    );
}

/// M1. Started today, credit added after the start, cancelled at once: the day's usage up to the
/// cancellation is paid by that credit.
#[tokio::test]
async fn a_subscription_cancelled_the_day_it_started_draws_credit_added_since() {
    let (env, usage) = usage_env().await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, -1).await;
    bandwidth_everywhere(&usage);
    let today = Utc::now().date_naive();
    let sub = start(&env, CUST_UBER_ID, PLAN_VERSION_USAGE_ONLY_ID, today).await;
    let goodwill = goodwill_now(&env).await;

    cancel_now(&env, sub).await;
    let last = usage_invoice(&env, sub, today, today).await;
    assert_eq!(money(&last), (3_000, 3_000, 0, 0, 0, 0));
    assert_eq!(
        movements(&env, goodwill).await,
        vec!["FUND 5000", "DRAW -3000"]
    );
}

/// M1. A EUR 100.00 top-up bought this morning, then an immediate cancellation: it was effective
/// before the cancellation, so it pays today's EUR 30.00 of usage.
#[tokio::test]
async fn a_top_up_bought_before_an_immediate_cancellation_pays_that_days_usage() {
    let (env, usage) = usage_env().await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, -1).await;
    bandwidth_everywhere(&usage);
    let today = Utc::now().date_naive();
    let sub = start(&env, CUST_UBER_ID, PLAN_VERSION_USAGE_ONLY_ID, today).await;
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("wallet");
    let sold = env
        .services()
        .sell_customer_credit(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 10_000,
                expires_in_months: None,
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
        )
        .await
        .expect("top-up sold")
        .invoice
        .invoice;
    pay(&env, &sold).await;
    let top_up = grant_of(&env, CUST_UBER_ID, sold.id, "PAID").await;
    assert!(top_up.effective_at > midnight(today));

    cancel_now(&env, sub).await;
    let last = usage_invoice(&env, sub, today, today).await;
    assert_eq!(money(&last), (3_000, 0, 0, 3_000, 3_000, 0));
    assert_eq!(
        movements(&env, top_up.id).await,
        vec!["FUND 10000", "DRAW -3000"]
    );
}

/// 5a. Usage-only plan from Jan 1, changed at once on Jan 16 to Large; EUR 50.00 added on Jan 20.
/// The renewal bills 100 units of the old plan (EUR 10.00, its lines ending Jan 16) and 200 of
/// the new (EUR 20.00): the credit, effective after the change, pays only the new plan's.
#[tokio::test]
async fn credit_effective_after_an_immediate_plan_change_pays_only_the_new_plans_usage() {
    let (env, usage) = usage_env().await;
    seed_usage_only_plan(&env).await;
    bandwidth_plan(&env, None).await;
    let sub = start(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_USAGE_ONLY_ID,
        date(2024, 1, 1),
    )
    .await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 100);
    set_usage(&usage, date(2024, 1, 16), date(2024, 2, 1), 200);
    change_on(&env, sub, LARGE, date(2024, 1, 16)).await;
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("wallet");
    let added = env
        .store()
        .fund_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 5_000,
                classification: GrantClassification::Paid,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Added after the change".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            midnight(date(2024, 1, 20)),
        )
        .await
        .expect("credit added")
        .grant_id;

    env.process_cycles().await;
    let old = usage_invoice(&env, sub, date(2024, 1, 1), date(2024, 1, 16)).await;
    let new = usage_invoice(&env, sub, date(2024, 1, 16), date(2024, 2, 1)).await;
    assert_eq!(old.id, new.id, "one renewal bills both plans' usage");
    assert_eq!(
        money(&new),
        (3_000, 0, 0, 3_000, 2_000, 1_000),
        "the old plan's EUR 10.00 is billed"
    );
    assert_eq!(
        movements(&env, added).await,
        vec!["FUND 5000", "DRAW -2000"]
    );
}

/// 5c. Large sells a EUR 500 pack of EUR 550 for three months on its Jan 1 invoice, paid only
/// after January ended: the pack is January's credit, so it pays January's EUR 80.00 of usage on
/// the renewal (bonus 50.00 before tax, then 30.00 of the paid credit).
#[tokio::test]
async fn a_plan_pack_paid_after_its_period_pays_that_periods_usage() {
    let (env, usage) = usage_env().await;
    bandwidth_plan(
        &env,
        Some(Pricing::Credit {
            unit_price: dec!(500),
            quantity: 1,
            credit_amount: dec!(550),
            expiry: Some(CreditExpiry::Month(3)),
            credit_unit_id: None,
            rollover: None,
            once: false,
        }),
    )
    .await;
    let sub = start(&env, CUST_UBER_ID, LARGE, date(2024, 1, 1)).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 2, 1), 800);
    let first = env.get_invoices(sub).await.remove(0);
    pay(&env, &first).await;
    let paid = grant_of(&env, CUST_UBER_ID, first.id, "PAID").await;
    let bonus = grant_of(&env, CUST_UBER_ID, first.id, "PROMOTIONAL").await;
    assert!(paid.effective_at > midnight(date(2024, 2, 1)), "paid since");
    assert_eq!(paid.pays_usage_from, Some(midnight(date(2024, 1, 1))));

    env.process_cycles().await;
    let renewal = usage_invoice(&env, sub, date(2024, 1, 1), date(2024, 2, 1)).await;
    assert_eq!(
        money(&renewal),
        (58_000, 5_000, 0, 53_000, 3_000, 50_000),
        "February's pack is due; January's usage is paid by January's pack"
    );
    assert_eq!(
        movements(&env, bonus.id).await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    assert_eq!(
        movements(&env, paid.id).await,
        vec!["FUND 50000", "DRAW -3000"]
    );
}
