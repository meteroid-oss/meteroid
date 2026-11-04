use std::collections::BTreeMap;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;
use chrono::{Datelike, NaiveDate, NaiveDateTime, Utc};
use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, PlanId, PlanVersionId, SubscriptionId};
use common_grpc::meteroid::common::v1::Date;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::bi::BiMrrMovementLogRowNew;
use diesel_models::customers::CustomerRowNew;
use diesel_models::enums::{CreditNoteStatus, MrrMovementType as DbMovementType};
use diesel_models::enums::{CustomerTaxStatusEnum, CustomerTypeEnum};
use diesel_models::historical_rates_from_usd::HistoricalRatesFromUsdRowNew;
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::stats::v1::{BreakdownStat, general_stats_response};
use meteroid_store::domain::credit::TopUpWallet;
use meteroid_store::domain::enums::{InvoiceStatusEnum, TaxResolverEnum};
use meteroid_store::domain::stats::{
    MrrMovementType, MrrTimeseriesPointRow, MrrTimeseriesRows, RevenueDailyPointRow,
};
use meteroid_store::domain::{Address, InvoicingEntityPatch};
use meteroid_store::domain::{CreditNote, Invoice};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::revenue_stats::postgres_revenue_stats;
use rust_decimal_macros::dec;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn test_stats_basic() {
    // Generic setup
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    // general stats
    let res = clients
        .stats
        .clone()
        .general_stats(api::stats::v1::GeneralStatsRequest {})
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        &res.total_mrr,
        &Some(general_stats_response::TotalMrr { value_cents: 0 })
    );

    // total_mrr_chart
    let res = clients
        .stats
        .clone()
        .total_mrr_chart(api::stats::v1::MrrChartRequest {
            start_date: None,
            end_date: None,
            plans_id: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .series;

    // With MINIMAL seed and no plans filter, we get one aggregated series
    assert_eq!(res.len(), 1);

    // mrr_breakdown
    let res = clients
        .stats
        .clone()
        .mrr_breakdown(api::stats::v1::MrrBreakdownRequest {
            start_date: None,
            end_date: None,
        })
        .await
        .unwrap()
        .into_inner()
        .mmr_breakdown
        .unwrap();

    assert_eq!(res.total_net_mrr, 0);

    // mrr_log
    let res = clients
        .stats
        .clone()
        .mrr_log(api::stats::v1::MrrLogRequest {
            before: None,
            after: None,
        })
        .await
        .unwrap()
        .into_inner()
        .entries;

    assert!(res.is_empty());

    // signup_sparkline
    let res = clients
        .stats
        .clone()
        .signup_sparkline(api::stats::v1::SignupSparklineRequest {})
        .await
        .unwrap()
        .into_inner()
        .series
        .unwrap();

    assert!(!res.data.is_empty());

    // trial_conversion_rate_sparkline
    let res = clients
        .stats
        .clone()
        .trial_conversion_rate_sparkline(api::stats::v1::TrialConversionRateSparklineRequest {})
        .await
        .unwrap()
        .into_inner()
        .series;

    assert!(!res.is_empty());

    // top_revenue_by_customer
    let res = clients
        .stats
        .clone()
        .top_revenue_by_customer(api::stats::v1::TopRevenueByCustomerRequest { count: 1 })
        .await
        .unwrap()
        .into_inner()
        .revenue_by_customer;

    assert!(res.is_empty());

    // teardown
    // meteroid_it::container::terminate_meteroid(setup.token, &setup.join_handle).await
}

/// Opening MRR is every movement before the range; each day in it sums its movements by type.
#[tokio::test]
async fn test_pg_mrr_timeseries_opening_and_daily_movements() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let stats = postgres_revenue_stats(env.store());

    let sub_a = create_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        date(2024, 1, 1),
        false,
    )
    .await;
    let sub_b = create_subscription(
        &env,
        CUST_SPOTIFY_ID,
        PLAN_VERSION_STARTER_ID,
        date(2024, 2, 10),
        false,
    )
    .await;
    let sub_c = create_subscription(
        &env,
        CUST_COMODO_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        date(2024, 2, 20),
        false,
    )
    .await;
    // Each creation books its new business on the start date.
    assert_eq!(
        creation_movement(&env, sub_a).await,
        (date(2024, 1, 1), 3_500)
    );
    assert_eq!(
        creation_movement(&env, sub_b).await,
        (date(2024, 2, 10), 3_900)
    );
    assert_eq!(
        creation_movement(&env, sub_c).await,
        (date(2024, 2, 20), 3_500)
    );

    let leetcode = PLAN_VERSION_1_LEETCODE_ID;
    insert_movement(
        &env,
        sub_a,
        leetcode,
        DbMovementType::Contraction,
        -200,
        "EUR",
        date(2024, 1, 15),
    )
    .await;
    insert_movement(
        &env,
        sub_b,
        PLAN_VERSION_STARTER_ID,
        DbMovementType::Expansion,
        1_000,
        "EUR",
        date(2024, 2, 10),
    )
    .await;
    insert_movement(
        &env,
        sub_c,
        leetcode,
        DbMovementType::Contraction,
        -500,
        "EUR",
        date(2024, 2, 20),
    )
    .await;
    insert_movement(
        &env,
        sub_a,
        leetcode,
        DbMovementType::Churn,
        -3_500,
        "EUR",
        date(2024, 2, 29),
    )
    .await;
    insert_movement(
        &env,
        sub_a,
        leetcode,
        DbMovementType::Reactivation,
        3_500,
        "EUR",
        date(2024, 3, 5),
    )
    .await;

    let (from, to) = (date(2024, 2, 1), date(2024, 2, 29));
    let rows = stats
        .mrr_timeseries(TENANT_ID, from, to, &[], false, "EUR")
        .await
        .unwrap();
    assert_eq!(
        opening(&rows),
        vec![(None, 3_300)],
        "35.00 less 2.00, both before the range"
    );
    assert_eq!(
        points(&rows),
        vec![
            Point::new(date(2024, 2, 10), None)
                .new_business(3_900, 1)
                .expansion(1_000, 1),
            Point::new(date(2024, 2, 20), None)
                .new_business(3_500, 1)
                .contraction(-500, 1),
            Point::new(date(2024, 2, 29), None).churn(-3_500, 1),
        ],
        "the range end is inclusive and the reactivation after it is left out"
    );

    let split = stats
        .mrr_timeseries(
            TENANT_ID,
            from,
            to,
            &[PLAN_LEETCODE_ID, PLAN_STARTER_ID],
            true,
            "EUR",
        )
        .await
        .unwrap();
    assert_eq!(opening(&split), vec![(Some(PLAN_LEETCODE_ID), 3_300)]);
    assert_eq!(
        points(&split),
        vec![
            Point::new(date(2024, 2, 10), Some(PLAN_STARTER_ID))
                .new_business(3_900, 1)
                .expansion(1_000, 1),
            Point::new(date(2024, 2, 20), Some(PLAN_LEETCODE_ID))
                .new_business(3_500, 1)
                .contraction(-500, 1),
            Point::new(date(2024, 2, 29), Some(PLAN_LEETCODE_ID)).churn(-3_500, 1),
        ]
    );

    let starter_only = stats
        .mrr_timeseries(TENANT_ID, from, to, &[PLAN_STARTER_ID], false, "EUR")
        .await
        .unwrap();
    assert!(starter_only.opening.is_empty());
    assert_eq!(
        points(&starter_only),
        vec![
            Point::new(date(2024, 2, 10), None)
                .new_business(3_900, 1)
                .expansion(1_000, 1)
        ]
    );

    let clients = grpc_clients(&env).await;
    let breakdown = clients
        .stats
        .clone()
        .mrr_breakdown(api::stats::v1::MrrBreakdownRequest {
            start_date: Some(proto_date(from)),
            end_date: Some(proto_date(to)),
        })
        .await
        .unwrap()
        .into_inner()
        .mmr_breakdown
        .unwrap();
    let stat = |count, value| Some(BreakdownStat { count, value });
    assert_eq!(breakdown.new_business, stat(2, 7_400));
    assert_eq!(breakdown.expansion, stat(1, 1_000));
    assert_eq!(breakdown.contraction, stat(1, -500));
    assert_eq!(breakdown.churn, stat(1, -3_500));
    assert_eq!(breakdown.reactivation, stat(0, 0));
    assert_eq!(breakdown.net_new_mrr, 4_400);

    let general = clients
        .stats
        .clone()
        .general_stats(api::stats::v1::GeneralStatsRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        general.total_mrr,
        Some(general_stats_response::TotalMrr {
            value_cents: 11_200
        }),
        "every movement applies before today"
    );
}

/// An issued invoice counts on its invoice date and a credit note on its finalization date. A
/// draft never counts, and a void nets to zero: its note reverses the invoice.
#[tokio::test]
async fn test_pg_revenue_daily_nets_credit_notes_and_voids() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let stats = postgres_revenue_stats(env.store());

    let starter_start = date(2024, 3, 1);
    let starter = create_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_STARTER_ID,
        starter_start,
        true,
    )
    .await;
    let starter_invoice = creation_invoice(&env, starter, starter_start, 3_900).await;
    let credited_on = credit_seats_line(&env, &starter_invoice).await;

    let voided_start = date(2024, 3, 5);
    let voided = create_subscription(
        &env,
        CUST_COMODO_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        voided_start,
        true,
    )
    .await;
    let voided_invoice = creation_invoice(&env, voided, voided_start, 3_500).await;
    env.try_void_invoice(voided_invoice.id)
        .await
        .expect("void the invoice");
    let (voided_on, void_subtotal) = void_note(&env, voided_invoice.id).await;
    assert_eq!(void_subtotal, -3_500, "the void credits the whole invoice");

    // A draft already carries its computed totals.
    let draft_date = date(2024, 4, 1);
    let draft = env.insert_draft_invoice(starter, draft_date).await;
    set_invoice_column(&env, draft.id, "subtotal = 50000").await;

    let (from, to) = (date(2024, 2, 1), credited_on.max(voided_on).max(draft_date));
    let revenue = |plans: Vec<PlanId>| {
        let stats = stats.clone();
        async move {
            daily(
                &stats
                    .revenue_daily(TENANT_ID, from, to, &plans, "EUR")
                    .await
                    .unwrap(),
            )
        }
    };

    assert_eq!(
        revenue(vec![]).await,
        by_day(&[
            (starter_start, 3_900),
            (credited_on, -1_000),
            (voided_start, 3_500),
            (voided_on, -3_500),
        ])
    );
    assert_eq!(
        revenue(vec![PLAN_STARTER_ID]).await,
        by_day(&[(starter_start, 3_900), (credited_on, -1_000)])
    );
    assert_eq!(
        revenue(vec![PLAN_LEETCODE_ID]).await,
        by_day(&[(voided_start, 3_500), (voided_on, -3_500)])
    );
}

/// A consolidated invoice counts once, not again through the per-subscription drafts it merged.
#[tokio::test]
async fn test_pg_revenue_daily_counts_a_consolidated_invoice_once() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    env.set_consolidate_recurring_invoices(true).await;
    let stats = postgres_revenue_stats(env.store());

    for _ in 0..2 {
        create_subscription(
            &env,
            CUST_UBER_ID,
            PLAN_VERSION_1_LEETCODE_ID,
            date(2024, 1, 1),
            false,
        )
        .await;
    }
    // Both renewals fall due the same day and merge into one invoice.
    env.process_cycles().await;

    let issued: Vec<Invoice> = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|i| i.status == InvoiceStatusEnum::Finalized)
        .collect();
    assert_eq!(
        issued.len(),
        3,
        "two creation invoices and the consolidated renewal"
    );
    let parent = issued
        .iter()
        .find(|i| i.subscription_id.is_none())
        .expect("the consolidated invoice");
    assert_eq!(parent.subtotal, 7_000);
    let children = env.get_consolidated_children(parent.id).await;
    assert_eq!(children.len(), 2);
    assert!(
        children
            .iter()
            .all(|c| c.status == InvoiceStatusEnum::Draft && c.subtotal == 3_500)
    );
    assert!(
        issued
            .iter()
            .filter(|i| i.subscription_id.is_some())
            .all(|i| i.subtotal == 3_500)
    );

    let expected = by_day(
        &issued
            .iter()
            .map(|i| (i.invoice_date, i.subtotal))
            .collect::<Vec<_>>(),
    );
    assert_eq!(expected.iter().map(|(_, net)| net).sum::<i64>(), 14_000);
    let (from, to) = (
        date(2024, 1, 1),
        issued.iter().map(|i| i.invoice_date).max().unwrap(),
    );
    let all = stats
        .revenue_daily(TENANT_ID, from, to, &[], "EUR")
        .await
        .unwrap();
    assert_eq!(daily(&all), expected);
    // The consolidated invoice carries no plan: a filter weighs it by its children's plans.
    let leetcode = stats
        .revenue_daily(TENANT_ID, from, to, &[PLAN_LEETCODE_ID], "EUR")
        .await
        .unwrap();
    assert_eq!(daily(&leetcode), expected);
    let starter = stats
        .revenue_daily(TENANT_ID, from, to, &[PLAN_STARTER_ID], "EUR")
        .await
        .unwrap();
    assert!(daily(&starter).is_empty(), "no child is on Starter");
}

/// A consolidated invoice gathering two plans splits between them by its children's subtotals.
#[tokio::test]
async fn test_pg_revenue_daily_splits_a_consolidated_invoice_by_plan() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    env.set_consolidate_recurring_invoices(true).await;
    let stats = postgres_revenue_stats(env.store());

    let start = date(2024, 1, 1);
    let leetcode_sub =
        create_subscription(&env, CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, start, false).await;
    let starter_sub =
        create_subscription(&env, CUST_UBER_ID, PLAN_VERSION_STARTER_ID, start, false).await;
    env.process_cycles().await;

    let issued: Vec<Invoice> = env
        .get_customer_invoices(CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|i| i.status == InvoiceStatusEnum::Finalized)
        .collect();
    assert_eq!(
        issued.len(),
        3,
        "two creation invoices and the consolidated renewal"
    );
    let issued_for = |sub: SubscriptionId| {
        issued
            .iter()
            .find(|i| i.subscription_id == Some(sub))
            .expect("the creation invoice")
    };
    let (leetcode_invoice, starter_invoice) = (issued_for(leetcode_sub), issued_for(starter_sub));
    assert_eq!(
        (leetcode_invoice.subtotal, starter_invoice.subtotal),
        (3_500, 3_900)
    );
    let parent = issued
        .iter()
        .find(|i| i.subscription_id.is_none())
        .expect("the consolidated invoice");
    assert_eq!((parent.total, parent.tax_amount), (7_400, 0));
    let mut children: Vec<i64> = env
        .get_consolidated_children(parent.id)
        .await
        .iter()
        .map(|c| c.subtotal)
        .collect();
    children.sort_unstable();
    assert_eq!(children, vec![3_500, 3_900]);

    let (from, to) = (start, issued.iter().map(|i| i.invoice_date).max().unwrap());
    let revenue = |plans: Vec<PlanId>| {
        let stats = stats.clone();
        async move {
            daily(
                &stats
                    .revenue_daily(TENANT_ID, from, to, &plans, "EUR")
                    .await
                    .unwrap(),
            )
        }
    };
    // 74.00 × 35/74 and 74.00 × 39/74.
    assert_eq!(
        revenue(vec![PLAN_LEETCODE_ID]).await,
        by_day(&[
            (leetcode_invoice.invoice_date, 3_500),
            (parent.invoice_date, 3_500)
        ])
    );
    assert_eq!(
        revenue(vec![PLAN_STARTER_ID]).await,
        by_day(&[
            (starter_invoice.invoice_date, 3_900),
            (parent.invoice_date, 3_900)
        ])
    );
    let everything = by_day(&[
        (leetcode_invoice.invoice_date, 3_500),
        (starter_invoice.invoice_date, 3_900),
        (parent.invoice_date, 7_400),
    ]);
    assert_eq!(
        revenue(vec![PLAN_LEETCODE_ID, PLAN_STARTER_ID]).await,
        everything
    );
    assert_eq!(revenue(vec![]).await, everything);
    assert!(revenue(vec![PLAN_NOTION_ID]).await.is_empty());
}

/// Revenue is net of coupon discounts.
#[tokio::test]
async fn test_pg_revenue_daily_counts_invoices_net_of_coupons() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let stats = postgres_revenue_stats(env.store());

    let percent = env.create_percentage_coupon("STATS10", 10).await;
    let fixed = env.create_fixed_coupon("STATS5", 5, "EUR").await;
    let leetcode = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(date(2024, 7, 1))
        .on_start()
        .no_trial()
        .coupon(percent)
        .create(env.services())
        .await;
    let starter = subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(date(2024, 7, 1))
        .on_start()
        .no_trial()
        .coupon(fixed)
        .create(env.services())
        .await;

    let leetcode_invoice = only_invoice(&env, leetcode).await;
    assert_eq!(
        (
            leetcode_invoice.subtotal,
            leetcode_invoice.discount,
            leetcode_invoice.tax_amount,
            leetcode_invoice.total
        ),
        (3_500, 350, 0, 3_150)
    );
    let starter_invoice = only_invoice(&env, starter).await;
    assert_eq!(
        (
            starter_invoice.subtotal,
            starter_invoice.discount,
            starter_invoice.tax_amount,
            starter_invoice.total
        ),
        (3_900, 500, 0, 3_400)
    );

    let to = leetcode_invoice
        .invoice_date
        .max(starter_invoice.invoice_date);
    let rows = stats
        .revenue_daily(TENANT_ID, date(2024, 7, 1), to, &[], "EUR")
        .await
        .unwrap();
    assert_eq!(
        daily(&rows),
        by_day(&[
            (leetcode_invoice.invoice_date, 3_150),
            (starter_invoice.invoice_date, 3_400)
        ])
    );
}

/// Revenue is net of tax, whether the plan quotes prices with or without it.
#[tokio::test]
async fn test_pg_revenue_daily_counts_invoices_net_of_tax() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let stats = postgres_revenue_stats(env.store());

    // The French entity's EU VAT resolver charges a French consumer the domestic 20%.
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .unwrap();
    let customer = create_french_consumer(&env).await;

    let exclusive_start = date(2024, 6, 3);
    let exclusive = create_subscription(
        &env,
        customer,
        PLAN_VERSION_1_LEETCODE_ID,
        exclusive_start,
        true,
    )
    .await;
    let exclusive_invoice = only_invoice(&env, exclusive).await;
    assert_eq!(exclusive_invoice.invoice_date, exclusive_start);
    assert_eq!(
        (exclusive_invoice.tax_amount, exclusive_invoice.total),
        (700, 4_200),
        "20% on top of 35.00"
    );

    set_leetcode_tax_inclusive(&env).await;
    let inclusive_start = date(2024, 6, 10);
    let inclusive = create_subscription(
        &env,
        customer,
        PLAN_VERSION_1_LEETCODE_ID,
        inclusive_start,
        true,
    )
    .await;
    let inclusive_invoice = only_invoice(&env, inclusive).await;
    assert_eq!(inclusive_invoice.invoice_date, inclusive_start);
    assert_eq!(
        (inclusive_invoice.tax_amount, inclusive_invoice.total),
        (583, 3_500),
        "35.00 holding 20%: 29.17 + 5.83"
    );

    let rows = stats
        .revenue_daily(TENANT_ID, date(2024, 6, 1), date(2024, 6, 30), &[], "EUR")
        .await
        .unwrap();
    assert_eq!(
        daily(&rows),
        vec![(exclusive_start, 3_500), (inclusive_start, 2_917)]
    );
}

/// What purchased credit paid counted when the credit was sold: the invoice it paid counts only
/// the rest, and so does a credit note on it. The purchase counts on its own date, without a plan.
#[tokio::test]
async fn test_pg_revenue_daily_counts_the_cash_share_of_credit_paid_invoices() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    env.seed_payments().await;
    let stats = postgres_revenue_stats(env.store());

    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .unwrap();
    let receipt = env
        .services()
        .top_up_wallet(
            TopUpWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 1_300,
                payment_method_id: CUST_UBER_PAYMENT_METHOD_ID,
                idempotency_key: "stats-top-up".to_owned(),
                expires_at: None,
                unit_price: None,
                expected_unit_price: None,
            },
            None,
        )
        .await
        .unwrap();
    for _ in 0..4 {
        run_once_invoice_orchestration(
            Arc::new(env.store().clone()),
            Arc::new(env.services().clone()),
        )
        .await;
    }
    let purchase = env.get_detailed_invoice(receipt.invoice_id).await.invoice;
    assert_eq!(purchase.status, InvoiceStatusEnum::Finalized);
    assert_eq!(
        (
            purchase.total,
            purchase.tax_amount,
            purchase.applied_credits
        ),
        (1_300, 0, 0)
    );

    // Issued now, so it draws the credit bought a moment ago.
    let sub = create_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_STARTER_ID,
        date(2024, 8, 1),
        false,
    )
    .await;
    let invoice = only_invoice(&env, sub).await;
    assert_eq!(
        (invoice.total, invoice.applied_credits),
        (3_900, 1_300),
        "the credit pays a third"
    );
    let credited_on = credit_seats_line(&env, &invoice).await;
    assert_eq!(
        env.get_detailed_invoice(invoice.id)
            .await
            .invoice
            .applied_credits,
        1_300
    );

    let to = purchase
        .invoice_date
        .max(invoice.invoice_date)
        .max(credited_on);
    let from = date(2024, 8, 1);
    let cash_share = [(invoice.invoice_date, 2_600), (credited_on, -667)];
    let rows = stats
        .revenue_daily(TENANT_ID, from, to, &[], "EUR")
        .await
        .unwrap();
    let mut all = cash_share.to_vec();
    all.push((purchase.invoice_date, 1_300));
    assert_eq!(daily(&rows), by_day(&all), "10.00 × 2/3 = 6.666… → 6.67");
    let starter = stats
        .revenue_daily(TENANT_ID, from, to, &[PLAN_STARTER_ID], "EUR")
        .await
        .unwrap();
    assert_eq!(
        daily(&starter),
        by_day(&cash_share),
        "the purchase has no plan"
    );
}

/// The rates in force on each date come from one lookup: the latest row on or before the date,
/// none before the first row.
#[tokio::test]
async fn test_find_historical_rates_by_dates_returns_the_rate_in_force() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;

    // The lookup is cached per date for the whole process: no other test uses 2003-2005.
    insert_eur_rate(&env, date(2004, 3, 1), 0.8).await;
    insert_eur_rate(&env, date(2004, 9, 1), 0.75).await;

    let asked = [
        date(2003, 2, 11),
        date(2004, 3, 1),
        date(2004, 5, 20),
        date(2004, 5, 20),
        date(2004, 9, 1),
        date(2005, 1, 7),
    ];
    let expected = vec![
        (date(2004, 3, 1), Some((date(2004, 3, 1), 0.8))),
        (date(2004, 5, 20), Some((date(2004, 3, 1), 0.8))),
        (date(2004, 9, 1), Some((date(2004, 9, 1), 0.75))),
        (date(2005, 1, 7), Some((date(2004, 9, 1), 0.75))),
        (date(2003, 2, 11), None),
    ];
    // Twice: the second call reads what the first cached, unless other tests evicted it.
    for _ in 0..2 {
        let found = env
            .store()
            .find_historical_rates_by_dates(&asked)
            .await
            .unwrap();
        assert_eq!(found.len(), 4);
        let in_force: Vec<_> = expected
            .iter()
            .map(|(day, _)| {
                (
                    *day,
                    found
                        .get(day)
                        .map(|r| (r.date, r.rates.get("EUR").copied().unwrap_or_default())),
                )
            })
            .collect();
        assert_eq!(in_force, expected);
    }
}

/// An amount in another currency converts at the latest rate on or before its own date; the
/// reporting currency is left as is.
#[tokio::test]
async fn test_pg_stats_convert_each_date_at_its_rate() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let stats = postgres_revenue_stats(env.store());

    // Before anything reads these dates: rates are cached per date for the whole process, and
    // no other test uses 2013, so that cache cannot hold another database's answer.
    insert_eur_rate(&env, date(2013, 4, 1), 0.5).await;
    insert_eur_rate(&env, date(2013, 6, 1), 0.75).await;

    let usd_start = date(2013, 4, 10);
    let usd_sub = create_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        usd_start,
        true,
    )
    .await;
    let usd_invoice = creation_invoice(&env, usd_sub, usd_start, 3_500).await;
    set_invoice_column(&env, usd_invoice.id, "currency = 'USD'").await;
    let eur_start = date(2013, 6, 3);
    let eur_sub = create_subscription(
        &env,
        CUST_SPOTIFY_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        eur_start,
        true,
    )
    .await;
    creation_invoice(&env, eur_sub, eur_start, 3_500).await;

    let usd = PLAN_VERSION_USD_ID;
    insert_movement(
        &env,
        usd_sub,
        usd,
        DbMovementType::Expansion,
        400,
        "USD",
        date(2013, 4, 12),
    )
    .await;
    insert_movement(
        &env,
        usd_sub,
        usd,
        DbMovementType::Expansion,
        1_000,
        "USD",
        date(2013, 4, 17),
    )
    .await;
    insert_movement(
        &env,
        usd_sub,
        usd,
        DbMovementType::Contraction,
        -2_000,
        "USD",
        date(2013, 6, 3),
    )
    .await;

    let rows = stats
        .mrr_timeseries(
            TENANT_ID,
            date(2013, 4, 15),
            date(2013, 6, 30),
            &[],
            false,
            "EUR",
        )
        .await
        .unwrap();
    assert_eq!(opening(&rows), vec![(None, 3_500 + 200)], "4.00 USD at 0.5");
    assert_eq!(
        points(&rows),
        vec![
            Point::new(date(2013, 4, 17), None).expansion(500, 1),
            Point::new(date(2013, 6, 3), None)
                .new_business(3_500, 1)
                .contraction(-1_500, 1),
        ]
    );

    let (from, to) = (date(2013, 4, 1), date(2013, 6, 30));
    let in_eur = stats
        .revenue_daily(TENANT_ID, from, to, &[], "EUR")
        .await
        .unwrap();
    assert_eq!(daily(&in_eur), vec![(usd_start, 1_750), (eur_start, 3_500)]);
    let in_usd = stats
        .revenue_daily(TENANT_ID, from, to, &[], "USD")
        .await
        .unwrap();
    assert_eq!(
        daily(&in_usd),
        vec![(usd_start, 3_500), (eur_start, 4_667)],
        "35.00 EUR at 0.75 is 46.666… USD"
    );
}

/// The log lists the latest movements first, each in its own currency and with its type.
#[tokio::test]
async fn test_pg_mrr_log_lists_latest_movements_first() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let stats = postgres_revenue_stats(env.store());

    let uber = create_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        date(2024, 5, 1),
        false,
    )
    .await;
    let spotify = create_subscription(
        &env,
        CUST_SPOTIFY_ID,
        PLAN_VERSION_STARTER_ID,
        date(2024, 5, 10),
        false,
    )
    .await;
    let downgrade = insert_movement(
        &env,
        uber,
        PLAN_VERSION_USD_ID,
        DbMovementType::Contraction,
        -700,
        "USD",
        date(2024, 5, 20),
    )
    .await;

    // Fixed write times, so the order does not hang on the clock between the inserts.
    let at = |day: u32| date(2024, 5, day).and_hms_opt(10, 0, 0).unwrap();
    set_movement_created_at(&env, creation_movement_id(&env, uber).await, at(1)).await;
    set_movement_created_at(&env, creation_movement_id(&env, spotify).await, at(10)).await;
    set_movement_created_at(&env, downgrade, at(20)).await;

    let line = |created: NaiveDateTime,
                applies_to: NaiveDate,
                customer: (CustomerId, &str),
                sub: SubscriptionId,
                plan: &str,
                kind: &'static str,
                change: i64,
                currency: &str| LogLine {
        created_at: created,
        applies_to,
        customer_id: customer.0.to_string(),
        customer_name: customer.1.to_string(),
        subscription_id: sub.to_string(),
        plan_name: plan.to_string(),
        kind,
        change,
        currency: currency.to_string(),
    };
    let downgrade_line = line(
        at(20),
        date(2024, 5, 20),
        (CUST_UBER_ID, "Uber"),
        uber,
        "USD Plan",
        "contraction",
        -700,
        "USD",
    );
    let spotify_line = line(
        at(10),
        date(2024, 5, 10),
        (CUST_SPOTIFY_ID, "Spotify"),
        spotify,
        "Starter",
        "new_business",
        3_900,
        "EUR",
    );
    let uber_line = line(
        at(1),
        date(2024, 5, 1),
        (CUST_UBER_ID, "Uber"),
        uber,
        "LeetCode",
        "new_business",
        3_500,
        "EUR",
    );

    let entries = stats.mrr_log(TENANT_ID, 10, None).await.unwrap();
    assert_eq!(entries[0].description, MANUAL_MOVEMENT);
    assert_eq!(
        log_lines(&entries),
        vec![downgrade_line.clone(), spotify_line.clone(), uber_line]
    );

    let latest = stats.mrr_log(TENANT_ID, 2, None).await.unwrap();
    assert_eq!(
        log_lines(&latest),
        vec![downgrade_line.clone(), spotify_line.clone()]
    );

    let window = stats
        .mrr_log(TENANT_ID, 10, Some((date(2024, 5, 5), date(2024, 5, 31))))
        .await
        .unwrap();
    assert_eq!(log_lines(&window), vec![downgrade_line, spotify_line]);
}

/// Customers rank by all-time billed net revenue; the year to date counts the current year only.
#[tokio::test]
async fn test_pg_top_revenue_by_customer() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let stats = postgres_revenue_stats(env.store());

    let setup_year = Utc::now().year();
    let this_year = date(setup_year, 1, 1);
    let (last_year_mid, last_year_early) =
        (date(setup_year - 1, 6, 15), date(setup_year - 1, 3, 1));

    let uber_old = create_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_STARTER_ID,
        last_year_mid,
        true,
    )
    .await;
    creation_invoice(&env, uber_old, last_year_mid, 3_900).await;
    let uber_new = create_subscription(
        &env,
        CUST_UBER_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        this_year,
        true,
    )
    .await;
    creation_invoice(&env, uber_new, this_year, 3_500).await;
    let comodo = create_subscription(
        &env,
        CUST_COMODO_ID,
        PLAN_VERSION_1_LEETCODE_ID,
        last_year_early,
        true,
    )
    .await;
    creation_invoice(&env, comodo, last_year_early, 3_500).await;
    let spotify = create_subscription(
        &env,
        CUST_SPOTIFY_ID,
        PLAN_VERSION_STARTER_ID,
        this_year,
        true,
    )
    .await;
    let spotify_invoice = creation_invoice(&env, spotify, this_year, 3_900).await;
    let credited_on = credit_seats_line(&env, &spotify_invoice).await;

    // In all-time order: 74.00, 35.00 (none this year), 29.00.
    type Billed = (CustomerId, &'static str, Vec<(NaiveDate, i64)>);
    let billed: [Billed; 3] = [
        (
            CUST_UBER_ID,
            "Uber",
            vec![(last_year_mid, 3_900), (this_year, 3_500)],
        ),
        (CUST_COMODO_ID, "Comodo", vec![(last_year_early, 3_500)]),
        (
            CUST_SPOTIFY_ID,
            "Spotify",
            vec![(this_year, 3_900), (credited_on, -1_000)],
        ),
    ];
    let expected = |year: i32| -> Vec<(String, String, i64, i64)> {
        billed
            .iter()
            .map(|(id, name, amounts)| {
                (
                    id.to_string(),
                    name.to_string(),
                    amounts.iter().map(|(_, cents)| cents).sum(),
                    amounts
                        .iter()
                        .filter(|(day, _)| day.year() == year)
                        .map(|(_, cents)| cents)
                        .sum(),
                )
            })
            .collect()
    };

    // The store reads the year itself: read again if the year turned around the call.
    let (top, year) = loop {
        let before = Utc::now().year();
        let top = stats
            .top_revenue_by_customer(TENANT_ID, 10, "EUR")
            .await
            .unwrap();
        if Utc::now().year() == before {
            break (top, before);
        }
    };
    assert!(top.iter().all(|c| c.currency == "EUR"));
    assert_eq!(
        top.iter()
            .map(|c| (
                c.customer_id.to_string(),
                c.customer_name.clone(),
                c.revenue_all_time,
                c.revenue_ytd
            ))
            .collect::<Vec<_>>(),
        expected(year)
    );

    let first_two = stats
        .top_revenue_by_customer(TENANT_ID, 2, "EUR")
        .await
        .unwrap();
    assert_eq!(
        first_two.iter().map(|c| c.customer_id).collect::<Vec<_>>(),
        vec![CUST_UBER_ID, CUST_COMODO_ID]
    );

    let clients = grpc_clients(&env).await;
    let (served, year) = loop {
        let before = Utc::now().year();
        let served = clients
            .stats
            .clone()
            .top_revenue_by_customer(api::stats::v1::TopRevenueByCustomerRequest { count: 2 })
            .await
            .unwrap()
            .into_inner()
            .revenue_by_customer;
        if Utc::now().year() == before {
            break (served, before);
        }
    };
    assert_eq!(
        served
            .into_iter()
            .map(|c| (
                c.customer_id,
                c.customer_name,
                c.revenue_all_time,
                c.revenue_ytd
            ))
            .collect::<Vec<_>>(),
        expected(year).into_iter().take(2).collect::<Vec<_>>()
    );
}

const MANUAL_MOVEMENT: &str = "Manual adjustment";

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn proto_date(day: NaiveDate) -> Date {
    Date {
        year: day.year(),
        month: day.month(),
        day: day.day(),
    }
}

async fn grpc_clients(env: &TestEnv) -> AllClients {
    let auth = meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    AllClients::from_channel(
        env.setup.channel.clone(),
        &auth.token,
        "TESTORG",
        "testslug",
    )
}

async fn create_subscription(
    env: &TestEnv,
    customer_id: CustomerId,
    plan_version_id: PlanVersionId,
    start: NaiveDate,
    backdated: bool,
) -> SubscriptionId {
    let builder = subscription()
        .customer(customer_id)
        .plan_version(plan_version_id)
        .start_date(start)
        .on_start()
        .no_trial();
    let builder = if backdated {
        builder.backdate_invoices()
    } else {
        builder
    };
    builder.create(env.services()).await
}

/// The subscription's only invoice, finalized at creation; backdated, it keeps the start date.
async fn creation_invoice(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    invoice_date: NaiveDate,
    subtotal: i64,
) -> Invoice {
    let invoice = only_invoice(env, subscription_id).await;
    assert_eq!(invoice.invoice_date, invoice_date);
    assert_eq!(invoice.subtotal, subtotal);
    invoice
}

/// The subscription's only invoice, finalized at creation.
async fn only_invoice(env: &TestEnv, subscription_id: SubscriptionId) -> Invoice {
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1, "only the creation invoice");
    let invoice = invoices.into_iter().next().unwrap();
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    invoice
}

/// A French consumer: no VAT number, so FR→FR domestic VAT applies.
async fn create_french_consumer(env: &TestEnv) -> CustomerId {
    let customer_id = CustomerId::new();
    let address = Address {
        line1: Some("12 rue de Rivoli".to_string()),
        line2: None,
        city: Some("Paris".to_string()),
        country: Some(CountryCode::from_str("FR").unwrap()),
        state: None,
        zip_code: Some("75004".to_string()),
    };
    let mut conn = env.conn().await;
    CustomerRowNew {
        id: customer_id,
        name: "Camille Durand".to_string(),
        created_at: Some(Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&address).unwrap()),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status: CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: false,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: CustomerTypeEnum::Individual,
        first_name: Some("Camille".to_string()),
        last_name: Some("Durand".to_string()),
    }
    .insert(&mut conn)
    .await
    .unwrap();
    customer_id
}

async fn set_leetcode_tax_inclusive(env: &TestEnv) {
    use diesel_models::schema::plan_version::dsl as pv;

    let mut conn = env.conn().await;
    diesel::update(pv::plan_version.filter(pv::id.eq(PLAN_VERSION_1_LEETCODE_ID)))
        .set(pv::tax_inclusive.eq(true))
        .execute(&mut conn)
        .await
        .unwrap();
}

/// Credits the 10.00 seat of a Starter invoice and returns the note's finalization day.
async fn credit_seats_line(env: &TestEnv, invoice: &Invoice) -> NaiveDate {
    let seat = invoice
        .line_items
        .iter()
        .find(|line| line.amount_subtotal == 1_000)
        .expect("the seat line");
    let draft = env
        .store()
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Line {
                    local_id: seat.local_id.clone(),
                    quantity: dec!(1),
                }],
                reason: None,
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .unwrap();
    let note: CreditNote = env
        .store()
        .finalize_credit_note(Actor::System, TENANT_ID, draft.id)
        .await
        .unwrap();
    assert_eq!(note.subtotal, -1_000, "credit notes are stored negative");
    note.finalized_at.expect("finalized").date()
}

/// The finalization day and subtotal of the note the invoice's void issued.
async fn void_note(env: &TestEnv, invoice_id: InvoiceId) -> (NaiveDate, i64) {
    use diesel_models::schema::credit_note::dsl as cn;

    let mut conn = env.conn().await;
    let (finalized_at, subtotal): (Option<NaiveDateTime>, i64) = cn::credit_note
        .filter(cn::invoice_id.eq(invoice_id))
        .filter(cn::voids_invoice.eq(true))
        .filter(cn::status.eq(CreditNoteStatus::Finalized))
        .select((cn::finalized_at, cn::subtotal))
        .first(&mut conn)
        .await
        .unwrap();
    (finalized_at.expect("finalized").date(), subtotal)
}

async fn set_invoice_column(env: &TestEnv, invoice_id: InvoiceId, assignment: &str) {
    let mut conn = env.conn().await;
    diesel::sql_query(format!("UPDATE invoice SET {assignment} WHERE id = $1"))
        .bind::<diesel::sql_types::Uuid, _>(*invoice_id)
        .execute(&mut conn)
        .await
        .unwrap();
}

async fn insert_eur_rate(env: &TestEnv, day: NaiveDate, eur_per_usd: f64) {
    let mut conn = env.conn().await;
    HistoricalRatesFromUsdRowNew::insert_batch(
        &mut conn,
        vec![HistoricalRatesFromUsdRowNew {
            id: Uuid::now_v7(),
            date: day,
            rates: serde_json::json!({ "USD": 1.0, "EUR": eur_per_usd }),
            updated_at: Utc::now().naive_utc(),
        }],
    )
    .await
    .unwrap();
}

async fn insert_movement(
    env: &TestEnv,
    subscription_id: SubscriptionId,
    plan_version_id: PlanVersionId,
    movement_type: DbMovementType,
    net_mrr_change: i64,
    currency: &str,
    applies_to: NaiveDate,
) -> Uuid {
    let customer_id = env.get_subscription(subscription_id).await.customer_id;
    let mut conn = env.conn().await;
    BiMrrMovementLogRowNew {
        id: Uuid::now_v7(),
        description: MANUAL_MOVEMENT.to_string(),
        movement_type,
        net_mrr_change,
        currency: currency.to_string(),
        applies_to,
        invoice_id: None,
        credit_note_id: None,
        plan_version_id,
        tenant_id: TENANT_ID,
        subscription_id,
        customer_id,
    }
    .insert(&mut conn)
    .await
    .unwrap()
    .id
}

/// The single movement the subscription's creation booked.
async fn creation_movement_row(
    env: &TestEnv,
    subscription_id: SubscriptionId,
) -> (Uuid, NaiveDate, i64) {
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;

    let mut conn = env.conn().await;
    let rows: Vec<(Uuid, NaiveDate, i64)> = m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(subscription_id))
        .filter(m::movement_type.eq(DbMovementType::NewBusiness))
        .select((m::id, m::applies_to, m::net_mrr_change))
        .load(&mut conn)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "one movement for the creation");
    rows[0]
}

async fn creation_movement(env: &TestEnv, subscription_id: SubscriptionId) -> (NaiveDate, i64) {
    let (_, applies_to, change) = creation_movement_row(env, subscription_id).await;
    (applies_to, change)
}

async fn creation_movement_id(env: &TestEnv, subscription_id: SubscriptionId) -> Uuid {
    creation_movement_row(env, subscription_id).await.0
}

async fn set_movement_created_at(env: &TestEnv, id: Uuid, created_at: NaiveDateTime) {
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;

    let mut conn = env.conn().await;
    diesel::update(m::bi_mrr_movement_log.filter(m::id.eq(id)))
        .set(m::created_at.eq(created_at))
        .execute(&mut conn)
        .await
        .unwrap();
}

#[derive(Debug, PartialEq)]
struct Point {
    date: NaiveDate,
    plan_id: Option<PlanId>,
    net: i64,
    new_business: (i64, i32),
    expansion: (i64, i32),
    contraction: (i64, i32),
    churn: (i64, i32),
    reactivation: (i64, i32),
}

/// Expected points sum their movements into `net`, so comparing checks the served net too.
impl Point {
    fn new(date: NaiveDate, plan_id: Option<PlanId>) -> Self {
        Point {
            date,
            plan_id,
            net: 0,
            new_business: (0, 0),
            expansion: (0, 0),
            contraction: (0, 0),
            churn: (0, 0),
            reactivation: (0, 0),
        }
    }

    fn new_business(mut self, cents: i64, count: i32) -> Self {
        self.net += cents;
        self.new_business = (cents, count);
        self
    }

    fn expansion(mut self, cents: i64, count: i32) -> Self {
        self.net += cents;
        self.expansion = (cents, count);
        self
    }

    fn contraction(mut self, cents: i64, count: i32) -> Self {
        self.net += cents;
        self.contraction = (cents, count);
        self
    }

    fn churn(mut self, cents: i64, count: i32) -> Self {
        self.net += cents;
        self.churn = (cents, count);
        self
    }
}

impl From<&MrrTimeseriesPointRow> for Point {
    fn from(row: &MrrTimeseriesPointRow) -> Self {
        Point {
            date: row.date,
            plan_id: row.plan_id,
            net: row.net_mrr_cents,
            new_business: (row.new_business_cents, row.new_business_count),
            expansion: (row.expansion_cents, row.expansion_count),
            contraction: (row.contraction_cents, row.contraction_count),
            churn: (row.churn_cents, row.churn_count),
            reactivation: (row.reactivation_cents, row.reactivation_count),
        }
    }
}

fn points(rows: &MrrTimeseriesRows) -> Vec<Point> {
    rows.points.iter().map(Point::from).collect()
}

fn opening(rows: &MrrTimeseriesRows) -> Vec<(Option<PlanId>, i64)> {
    rows.opening
        .iter()
        .map(|row| (row.plan_id, row.mrr_cents))
        .collect()
}

fn daily(rows: &[RevenueDailyPointRow]) -> Vec<(NaiveDate, i64)> {
    rows.iter()
        .map(|row| (row.date, row.net_revenue_cents))
        .collect()
}

/// Amounts summed per day, in date order: two amounts may land on the same day.
fn by_day(amounts: &[(NaiveDate, i64)]) -> Vec<(NaiveDate, i64)> {
    let mut days: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    for (day, cents) in amounts {
        *days.entry(*day).or_default() += cents;
    }
    days.into_iter().collect()
}

#[derive(Debug, PartialEq, Clone)]
struct LogLine {
    created_at: NaiveDateTime,
    applies_to: NaiveDate,
    customer_id: String,
    customer_name: String,
    subscription_id: String,
    plan_name: String,
    kind: &'static str,
    change: i64,
    currency: String,
}

fn log_lines(entries: &[meteroid_store::domain::stats::MrrLogEntry]) -> Vec<LogLine> {
    entries
        .iter()
        .map(|entry| LogLine {
            created_at: entry.created_at,
            applies_to: entry.applies_to,
            customer_id: entry.customer_id.clone(),
            customer_name: entry.customer_name.clone(),
            subscription_id: entry.subscription_id.clone(),
            plan_name: entry.plan_name.clone(),
            kind: match entry.mrr_type {
                MrrMovementType::NewBusiness => "new_business",
                MrrMovementType::Expansion => "expansion",
                MrrMovementType::Contraction => "contraction",
                MrrMovementType::Churn => "churn",
                MrrMovementType::Reactivation => "reactivation",
            },
            change: entry.mrr_change,
            currency: entry.currency.clone(),
        })
        .collect()
}
