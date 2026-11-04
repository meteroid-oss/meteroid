//! The dashboard's credit reads: per-wallet activity totals, per-grant consumption and the ledger
//! labelled by where each grant came from, over a pack, promotional credit, a draw, an expiry and a
//! rollover.
use std::sync::Arc;

use super::usage::{PLAN_VERSION_USAGE_ONLY_ID, seed_usage_only_plan};
use crate::data::ids::{CUST_UBER_ID, METRIC_BANDWIDTH, TENANT_ID};
use crate::harness::{TestEnv, subscription, test_env, test_env_with_seed_and_usage};
use crate::helpers::credit::{
    Grant, assert_ledger_balances, expire_grant, fund, fund_grant, fund_subscription_grant,
};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_customer_credit::{draft, finalize, line};
use chrono::{NaiveDate, NaiveTime, Utc};
use common_domain::ids::{CreditGrantId, SubscriptionId};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::LineItemKind;
use meteroid_store::domain::UsagePeriod;
use meteroid_store::domain::credit::{
    CreditGrantSource, CreditGrantState, GrantClassification, GrantRollover, LedgerEntryKind,
};
use meteroid_store::domain::price_components::CreditExpiry;
use meteroid_store::repositories::credit::activity::LabelledLedgerEntry;
use meteroid_store::repositories::credit::{
    CustomerCreditActivityInterface, CustomerCreditWalletsInterface,
};
use rstest::rstest;
use rust_decimal::Decimal;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await
}

/// A plan's monthly pack: included credit that rolls over in full at period end.
fn rolling_pack_line(amount: i64) -> meteroid_store::domain::LineItem {
    let base = line("Monthly credits", 0, false);
    meteroid_store::domain::LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(amount),
        credit_grant_included: true,
        start_date: base.start_date - chrono::Duration::days(2),
        credit_grant_expiry: Some(CreditExpiry::BillingPeriod),
        credit_grant_rollover: Some(GrantRollover {
            max_minor: None,
            expiry: CreditExpiry::Month(1),
        }),
        ..base
    }
}

/// The customer's single-wallet labelled statement: a rollover is one row that moves no balance,
/// every row's balance follows from the rows before it and ends at `available`, and pages and
/// the count cover the same rows.
async fn assert_labelled_statement(
    store: &meteroid_store::Store,
    available: i64,
) -> TestResult<Vec<LabelledLedgerEntry>> {
    let (entries, total) = store
        .customer_credit_labelled_ledger(TENANT_ID, CUST_UBER_ID, None, None, 1_000, 0)
        .await
        .map_err(test_error)?;
    assert_eq!(total as usize, entries.len());
    assert!(
        entries
            .iter()
            .all(|e| e.entry.kind != LedgerEntryKind::Rollover)
    );
    let mut balance = 0_i64;
    for (index, labelled) in entries.iter().rev().enumerate() {
        let carried = labelled.entry.kind == LedgerEntryKind::Fund
            && labelled.grant_source == CreditGrantSource::Rollover;
        if !carried {
            balance += labelled.entry.amount_minor;
        }
        assert_eq!(
            labelled.entry.balance_after_minor, balance,
            "balance after row {index}"
        );
        assert_eq!(labelled.entry.sequence, index as i64 + 1);
    }
    assert_eq!(balance, available);

    for (offset, limit) in [(0_usize, 2_usize), (2, 2), (4, 3)] {
        let (page, page_total) = store
            .customer_credit_labelled_ledger(
                TENANT_ID,
                CUST_UBER_ID,
                None,
                None,
                limit as i64,
                offset as i64,
            )
            .await
            .map_err(test_error)?;
        assert_eq!(page_total, total);
        let expected = entries
            .iter()
            .skip(offset)
            .take(limit)
            .map(|e| (e.entry.id, e.entry.balance_after_minor))
            .collect::<Vec<_>>();
        assert_eq!(
            page.iter()
                .map(|e| (e.entry.id, e.entry.balance_after_minor))
                .collect::<Vec<_>>(),
            expected
        );
    }
    Ok(entries)
}

#[tokio::test]
async fn credit_activity_and_labelled_ledger_follow_every_movement() -> TestResult {
    let setup = setup().await;
    setup.skip_credit_invariants(
        "moves a grant's expiry or a subscription's end into the past by raw SQL after credit was drawn",
    );
    let store = &setup.store;

    let pack = draft(
        &setup,
        vec![rolling_pack_line(2_000), line("Seats", 2_000, false)],
    )
    .await?;
    let pack = finalize(&setup, pack.id).await?.invoice;

    let goodwill = fund(store, TENANT_ID, CUST_UBER_ID, None, 1_000).await;
    let lapsing = fund_grant(
        store,
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + chrono::Duration::days(3)),
            ..Grant::invoice(500)
        },
    )
    .await;
    let included = store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .find(|grant| grant.source == CreditGrantSource::Included)
        .expect("the pack funds included credit")
        .id;

    // The pack expires first, so it pays the draw of its subscription's usage; included credit
    // is promotional, taken off before tax.
    let spend = draft(&setup, vec![line("Usage", 700, true)]).await?;
    let spend = finalize(&setup, spend.id).await?.invoice;
    assert_eq!((spend.promotional_credits, spend.applied_credits), (700, 0));

    expire_grant(store, included).await;
    expire_grant(store, lapsing).await;
    assert_eq!(
        store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        2
    );

    let wallets = store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    assert_eq!(wallets.len(), 1);
    let wallet = &wallets[0];
    let rolled = wallet
        .grants
        .iter()
        .find(|grant| grant.source == CreditGrantSource::Rollover)
        .expect("the pack rolled over");
    assert_eq!(rolled.remaining_minor, 1_300);
    assert_eq!(rolled.state, CreditGrantState::Active);

    let activity = store
        .customer_credit_activity(TENANT_ID, CUST_UBER_ID, None, None)
        .await
        .map_err(test_error)?;
    assert_eq!(activity.len(), 1);
    let activity = &activity[0];
    assert_eq!(activity.wallet_id, wallet.id);
    assert_eq!(
        activity.credit_added_minor, 3_500,
        "rolled-over credit is not added credit"
    );
    assert_eq!(activity.invoiced_minor, 700);
    assert_eq!(activity.expired_minor, 500);
    assert_eq!(activity.removed_minor, 0);
    assert_eq!(activity.settled_at_expiry_minor, 0);
    assert_eq!(activity.next_expiry, rolled.expires_at);

    let mut groups = activity
        .credit_added_groups
        .iter()
        .map(|g| {
            (
                g.source,
                g.classification,
                g.name.as_str(),
                g.grants,
                g.amount_minor,
            )
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|g| g.4);
    assert_eq!(
        groups,
        vec![
            (
                CreditGrantSource::Manual,
                GrantClassification::Paid,
                "Integration-test customer credit",
                2,
                1_500
            ),
            (
                CreditGrantSource::Included,
                GrantClassification::Promotional,
                "Monthly credits",
                1,
                2_000
            ),
        ]
    );

    let grant = |id| {
        activity
            .grants
            .iter()
            .find(|grant| grant.grant_id == id)
            .expect("every grant is listed")
    };
    let pack_activity = grant(included);
    assert_eq!(pack_activity.name, "Monthly credits");
    assert_eq!(pack_activity.invoiced_minor, 700);
    assert_eq!(pack_activity.settled_at_expiry_minor, 0);
    assert_eq!(pack_activity.rolled_over_minor, 1_300);
    assert_eq!(pack_activity.unbilled_minor, 0);
    assert_eq!(
        pack_activity.invoice_number.as_deref(),
        Some(pack.invoice_number.as_str())
    );
    assert!(!pack_activity.automatic_top_up);
    assert_eq!(grant(goodwill).invoiced_minor, 0);
    assert_eq!(grant(lapsing).invoiced_minor, 0);
    assert_eq!(grant(rolled.id).invoiced_minor, 0);

    // Since a later instant, only the movements recorded from then count.
    let later = store
        .customer_credit_activity(
            TENANT_ID,
            CUST_UBER_ID,
            Some(Utc::now() + chrono::Duration::days(1)),
            None,
        )
        .await
        .map_err(test_error)?;
    assert_eq!(later[0].credit_added_minor, 0);
    assert_eq!(later[0].invoiced_minor, 0);
    assert_eq!(later[0].grants.len(), wallet.grants.len());

    let entries = assert_labelled_statement(store, wallet.available_minor).await?;
    let total = entries.len() as u64;
    assert_eq!(
        entries.len(),
        6,
        "3 fundings, a draw, one rollover line and an expiry"
    );
    assert!(
        entries
            .iter()
            .all(|e| e.entry.kind != LedgerEntryKind::Rollover)
    );

    // Business-time order: the lapse is dated at the lapsing grant's funding (its expiry was moved
    // before it), so it precedes the draw: 2,000 + 1,000 + 500 − 500, then − 700.
    let lapse = &entries[entries.len() - 4];
    assert_eq!(lapse.entry.kind, LedgerEntryKind::Expiry);
    assert_eq!(lapse.entry.balance_after_minor, 3_000);
    let draw = &entries[entries.len() - 5];
    assert_eq!(draw.entry.kind, LedgerEntryKind::Draw);
    assert_eq!(draw.entry.balance_after_minor, 2_300);
    assert!(
        entries
            .iter()
            .all(|e| e.currency == wallet.currency && e.credit_unit.is_none())
    );
    assert_eq!(entries[0].entry.balance_after_minor, 2_300);
    assert_eq!(wallet.available_minor, 2_300);

    let find = |kind: LedgerEntryKind, source: CreditGrantSource| {
        entries
            .iter()
            .find(|e| e.entry.kind == kind && e.grant_source == source)
            .unwrap_or_else(|| panic!("a {kind:?} line of a {source:?} grant"))
    };
    let funded = find(LedgerEntryKind::Fund, CreditGrantSource::Included);
    assert_eq!(funded.entry.amount_minor, 2_000);
    assert_eq!(funded.grant_name, "Monthly credits");
    assert_eq!(
        funded.invoice_number.as_deref(),
        Some(pack.invoice_number.as_str())
    );
    assert_eq!(
        funded.grant_classification,
        GrantClassification::Promotional
    );

    let drawn = find(LedgerEntryKind::Draw, CreditGrantSource::Included);
    assert_eq!(drawn.entry.amount_minor, -700);
    assert_eq!(
        drawn.invoice_number.as_deref(),
        Some(spend.invoice_number.as_str())
    );

    let carried = find(LedgerEntryKind::Fund, CreditGrantSource::Rollover);
    assert_eq!(carried.entry.amount_minor, 1_300);
    assert!(
        carried.invoice_number.is_none()
            || carried.invoice_number.as_deref() == Some(pack.invoice_number.as_str())
    );

    let lapsed = find(LedgerEntryKind::Expiry, CreditGrantSource::Manual);
    assert_eq!(lapsed.entry.amount_minor, -500);
    assert_eq!(lapsed.entry.grant_id, lapsing);
    assert_eq!(
        entries
            .iter()
            .filter(|e| e.entry.kind == LedgerEntryKind::Fund
                && e.grant_source == CreditGrantSource::Manual)
            .count(),
        2
    );
    assert!(
        entries
            .iter()
            .all(|e| !e.automatic_top_up && e.grant_created_by.is_none())
    );

    // One wallet's statement is the same when asked for by id.
    let (only, only_total) = store
        .customer_credit_labelled_ledger(TENANT_ID, CUST_UBER_ID, Some(wallet.id), None, 3, 0)
        .await
        .map_err(test_error)?;
    assert_eq!(only_total, total);
    assert_eq!(
        only.iter().map(|e| e.entry.id).collect::<Vec<_>>(),
        entries
            .iter()
            .take(3)
            .map(|e| e.entry.id)
            .collect::<Vec<_>>()
    );

    assert_ledger_balances(store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// 0.10 EUR per unit on the usage-only plan: `units` units are `units * 10` cents.
fn set_january_usage(usage: &MutableMockUsageClient, units: i64) {
    let start = january().and_time(NaiveTime::MIN);
    let end = NaiveDate::from_ymd_opt(2024, 2, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

fn january() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
}

/// Credit that expires mid-period pays its usage before any invoice: that is settled at expiry,
/// not invoiced, until the period's invoice takes the settlement over.
#[tokio::test]
async fn credit_settled_at_expiry_is_not_invoiced_until_the_invoice_takes_it_over() -> TestResult {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let subscription_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(january())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycle_transitions().await;
    let pack = draft(
        &env.setup,
        vec![rolling_pack_line(2_000), line("Seats", 2_000, false)],
    )
    .await?;
    finalize(&env.setup, pack.id).await?;
    let included = env
        .store()
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .find(|grant| grant.source == CreditGrantSource::Included)
        .expect("the pack funds included credit")
        .id;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant {
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            ..Grant::invoice(5_000)
        },
    )
    .await;
    expire_grant(env.store(), grant).await;
    expire_grant(env.store(), included).await;
    set_january_usage(&usage, 100);
    // One sweep settles the usage, lapses the promotional credit and rolls the pack over.
    assert_eq!(
        env.store()
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        2
    );

    let activity = env
        .store()
        .customer_credit_activity(TENANT_ID, CUST_UBER_ID, None, None)
        .await
        .map_err(test_error)?
        .remove(0);
    assert_eq!(activity.credit_added_minor, 7_000);
    assert_eq!(activity.settled_at_expiry_minor, 1_000);
    assert_eq!(activity.invoiced_minor, 0);
    let rolled = activity
        .grants
        .iter()
        .map(|g| g.rolled_over_minor)
        .sum::<i64>();
    assert!(rolled > 0, "the pack rolled over in the sweep");
    assert_eq!(activity.expired_minor + rolled, 6_000);
    assert_eq!(
        activity
            .grants
            .iter()
            .map(|g| (g.settled_at_expiry_minor, g.invoiced_minor))
            .fold((0, 0), |(s, i), (gs, gi)| (s + gs, i + gi)),
        (1_000, 0)
    );

    let wallet = env
        .store()
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .remove(0);
    assert_eq!(wallet.available_minor, rolled);
    let entries = assert_labelled_statement(env.store(), wallet.available_minor).await?;
    let draws = entries
        .iter()
        .filter(|e| e.entry.kind == LedgerEntryKind::Draw)
        .collect::<Vec<_>>();
    assert_eq!(
        draws.iter().map(|e| e.entry.amount_minor).sum::<i64>(),
        -1_000
    );
    assert!(
        draws
            .iter()
            .all(|e| e.entry.invoice_id.is_none() && e.invoice_number.is_none())
    );
    let carried = entries
        .iter()
        .find(|e| {
            e.entry.kind == LedgerEntryKind::Fund && e.grant_source == CreditGrantSource::Rollover
        })
        .expect("one rollover row");
    assert_eq!(carried.entry.amount_minor, rolled);
    assert!(
        entries
            .iter()
            .any(|e| e.entry.kind == LedgerEntryKind::Expiry),
        "the promotional remainder lapsed in the same sweep"
    );

    env.process_due_events().await;
    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(invoice.applied_credits, 1_000);

    let activity = env
        .store()
        .customer_credit_activity(TENANT_ID, CUST_UBER_ID, None, None)
        .await
        .map_err(test_error)?
        .remove(0);
    assert_eq!(activity.settled_at_expiry_minor, 0);
    assert_eq!(activity.invoiced_minor, 1_000);
    let (entries, _) = env
        .store()
        .customer_credit_labelled_ledger(TENANT_ID, CUST_UBER_ID, None, None, 100, 0)
        .await
        .map_err(test_error)?;
    assert!(
        entries
            .iter()
            .filter(|e| e.entry.kind == LedgerEntryKind::Draw)
            .all(|e| e.entry.invoice_id == Some(invoice.id)
                && e.invoice_number.as_deref() == Some(invoice.invoice_number.as_str()))
    );
    let wallet = env
        .store()
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .remove(0);
    assert_labelled_statement(env.store(), wallet.available_minor).await?;
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

async fn live_subscription(env: &TestEnv) -> SubscriptionId {
    subscription()
        .customer(CUST_UBER_ID)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

/// The statement of the credit `subscription_id` can spend, checked whole: its sequence and running
/// balance count its own rows only, and pages cover the rows the count does.
async fn assert_scoped_statement(
    store: &meteroid_store::Store,
    subscription_id: SubscriptionId,
    spendable: &[CreditGrantId],
) -> TestResult<Vec<LabelledLedgerEntry>> {
    let (entries, total) = store
        .customer_credit_labelled_ledger(
            TENANT_ID,
            CUST_UBER_ID,
            None,
            Some(subscription_id),
            1_000,
            0,
        )
        .await
        .map_err(test_error)?;
    assert_eq!(total as usize, entries.len());
    assert!(
        entries
            .iter()
            .all(|e| spendable.contains(&e.entry.grant_id)),
        "only the subscription's own and the customer-wide credit"
    );
    let mut balance = 0_i64;
    for (index, labelled) in entries.iter().rev().enumerate() {
        balance += labelled.entry.amount_minor;
        assert_eq!(labelled.entry.balance_after_minor, balance);
        assert_eq!(labelled.entry.sequence, index as i64 + 1);
    }
    let (page, page_total) = store
        .customer_credit_labelled_ledger(TENANT_ID, CUST_UBER_ID, None, Some(subscription_id), 1, 1)
        .await
        .map_err(test_error)?;
    assert_eq!(page_total, total);
    assert_eq!(
        page.iter().map(|e| e.entry.id).collect::<Vec<_>>(),
        entries
            .iter()
            .skip(1)
            .take(1)
            .map(|e| e.entry.id)
            .collect::<Vec<_>>()
    );
    Ok(entries)
}

/// A subscription's view of the wallet: the movements of its own grants and of the customer-wide
/// ones, never another subscription's, in the statement and in the activity totals.
#[rstest]
#[tokio::test]
async fn a_subscription_sees_the_movements_of_the_credit_it_can_spend(#[future] test_env: TestEnv) {
    scoped_credit_views(test_env.await)
        .await
        .expect("the credit each subscription can spend");
}

async fn scoped_credit_views(env: TestEnv) -> TestResult {
    let store = env.store();
    let (a, b) = (live_subscription(&env).await, live_subscription(&env).await);
    let own_a =
        fund_subscription_grant(store, TENANT_ID, CUST_UBER_ID, a, Grant::invoice(1_000)).await;
    let own_b =
        fund_subscription_grant(store, TENANT_ID, CUST_UBER_ID, b, Grant::invoice(2_000)).await;
    let shared = fund(store, TENANT_ID, CUST_UBER_ID, None, 4_000).await;

    let usage_b = meteroid_store::domain::LineItem {
        subscription_id: Some(b),
        ..line("Usage B", 2_500, true)
    };
    let invoice = draft(&env.setup, vec![usage_b]).await?;
    let invoice = finalize(&env.setup, invoice.id).await?.invoice;
    assert_eq!(invoice.applied_credits, 2_500);
    let drawn = store
        .invoice_credit_by_grant(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?;
    let drawn_from = |grant| {
        drawn
            .iter()
            .filter(|(id, _)| *id == grant)
            .map(|(_, amount)| *amount)
            .sum::<i64>()
    };
    let (from_b, from_shared) = (drawn_from(own_b), drawn_from(shared));
    assert_eq!(from_b + from_shared, 2_500);

    let for_a = assert_scoped_statement(store, a, &[own_a, shared]).await?;
    assert_eq!(
        for_a.first().map(|e| e.entry.balance_after_minor),
        Some(1_000 + 4_000 - from_shared),
        "B's draws on shared credit are what A can no longer spend"
    );
    let for_b = assert_scoped_statement(store, b, &[own_b, shared]).await?;
    assert_eq!(
        for_b.first().map(|e| e.entry.balance_after_minor),
        Some(2_000 + 4_000 - 2_500)
    );
    let (all, all_total) = store
        .customer_credit_labelled_ledger(TENANT_ID, CUST_UBER_ID, None, None, 1_000, 0)
        .await
        .map_err(test_error)?;
    assert_eq!(all_total as usize, all.len());
    assert_eq!(
        all.first().map(|e| e.entry.balance_after_minor),
        Some(7_000 - 2_500)
    );
    assert!(all.len() > for_a.len() && all.len() > for_b.len());

    let activity = |subscription_id| async move {
        store
            .customer_credit_activity(TENANT_ID, CUST_UBER_ID, None, subscription_id)
            .await
            .map_err(test_error)
            .map(|mut wallets| wallets.remove(0))
    };
    let (of_a, of_b, of_all) = (
        activity(Some(a)).await?,
        activity(Some(b)).await?,
        activity(None).await?,
    );
    assert_eq!(
        (of_a.credit_added_minor, of_a.invoiced_minor),
        (5_000, from_shared)
    );
    assert_eq!(
        (of_b.credit_added_minor, of_b.invoiced_minor),
        (6_000, 2_500)
    );
    assert_eq!(
        (of_all.credit_added_minor, of_all.invoiced_minor),
        (7_000, 2_500)
    );
    let added = |activity: &meteroid_store::repositories::credit::activity::WalletActivity| {
        activity
            .credit_added_groups
            .iter()
            .map(|group| group.amount_minor)
            .sum::<i64>()
    };
    assert_eq!(
        (added(&of_a), added(&of_b), added(&of_all)),
        (5_000, 6_000, 7_000)
    );
    assert_eq!(
        of_a.grants.len(),
        of_all.grants.len(),
        "grants are listed whatever the subscription"
    );

    assert_ledger_balances(store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}
