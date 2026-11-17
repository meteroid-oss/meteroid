//! The billing controller: when a customer runs, what a run takes, and the sweep that
//! finds a lost wake.
use chrono::{NaiveDate, TimeZone, Utc};
use common_domain::ids::CustomerId;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_models::schema::billing_controller as bc;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

#[derive(Debug, Queryable)]
struct Controller {
    next_wake_at: Option<chrono::DateTime<Utc>>,
    dirty_since: Option<chrono::DateTime<Utc>>,
    lease_until: Option<chrono::DateTime<Utc>>,
}

async fn controller(env: &TestEnv, customer: CustomerId) -> Controller {
    let mut conn = env.conn().await;
    bc::table
        .filter(bc::tenant_id.eq(TENANT_ID))
        .filter(bc::customer_id.eq(customer))
        .select((bc::next_wake_at, bc::dirty_since, bc::lease_until))
        .first(&mut conn)
        .await
        .expect("the customer's billing controller")
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A new subscription marks its customer dirty; the run clears it and wakes it at the period end,
/// when the renewal is due.
#[rstest]
#[tokio::test]
async fn a_run_wakes_the_customer_when_its_renewal_is_due(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let before = controller(&env, CUST_UBER_ID).await;
    assert!(before.dirty_since.is_some(), "the subscription is an input");

    env.services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    let after = controller(&env, CUST_UBER_ID).await;
    assert_eq!(after.dirty_since, None);
    assert_eq!(after.lease_until, None);
    assert_eq!(
        after.next_wake_at,
        Some(Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap())
    );
}

/// A subscription three periods behind advances one period per run, running again at once until
/// it is current.
#[rstest]
#[tokio::test]
async fn a_subscription_behind_catches_up_one_period_per_run(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    let sub = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(Utc.with_ymd_and_hms(2024, 4, 15, 12, 0, 0).unwrap());

    let mut runs = 0;
    while env.get_subscription(sub).await.current_period_start < day(2024, 4, 1) {
        env.services()
            .sweep_billing_controller()
            .await
            .expect("the sweep wakes it");
        let run = env
            .services()
            .run_billing_controller(100)
            .await
            .expect("a run");
        assert_eq!(run.customers, 1);
        runs += 1;
        assert!(runs <= 3, "one period per run");
        // Behind, it runs again at once: its wake is the run's own start.
        if env.get_subscription(sub).await.current_period_start < day(2024, 4, 1) {
            let wake = controller(&env, CUST_UBER_ID).await.next_wake_at;
            assert!(wake.is_some_and(|at| at <= common_clock::now()));
        }
    }
    assert_eq!(runs, 3);
    assert_eq!(env.get_invoices(sub).await.len(), 4);
}

/// The sweep finds a customer due whose stored wake was not (a lost wake) and wakes it.
#[rstest]
#[tokio::test]
async fn the_sweep_wakes_a_customer_whose_wake_was_lost(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    {
        let mut conn = env.conn().await;
        diesel::update(bc::table.filter(bc::customer_id.eq(CUST_UBER_ID)))
            .set(bc::next_wake_at.eq(Some(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap())))
            .execute(&mut conn)
            .await
            .expect("a lost wake");
    }
    env.set_now(Utc.with_ymd_and_hms(2024, 2, 1, 6, 0, 0).unwrap());

    let sweep = env
        .services()
        .sweep_billing_controller()
        .await
        .expect("the sweep");
    assert_eq!(sweep.lost, 1);
    assert_eq!(
        controller(&env, CUST_UBER_ID).await.next_wake_at,
        Some(Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap())
    );
}

/// Credit of a customer with no subscription lapses too: the sweep finds the customer by its
/// credit, the run settles the expiry.
#[rstest]
#[tokio::test]
async fn credit_of_a_customer_without_a_subscription_lapses(#[future] test_env: TestEnv) {
    use diesel_models::schema::customer_credit_grant as g;
    use meteroid_testkit::helpers::credit::{Grant, expire_grant, fund_grant};
    let env = test_env.await;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_COMODO_ID,
        Some("EUR"),
        Grant::invoice(1_000),
    )
    .await;
    expire_grant(env.store(), grant).await;

    env.run_billing_controller().await;
    env.services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    let remaining: i64 = g::table
        .filter(g::id.eq(grant))
        .select(g::remaining)
        .first(&mut env.conn().await)
        .await
        .expect("the grant");
    assert_eq!(remaining, 0, "lapsed");
}

/// A run that failed waits for its retry, even with an input changed since.
#[rstest]
#[tokio::test]
async fn a_failed_run_waits_for_its_retry(#[future] test_env: TestEnv) {
    let env = test_env.await;
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    {
        let mut conn = env.conn().await;
        let now = common_clock::now();
        diesel_models::billing_controller::BillingControllerRow::fail(
            &mut conn,
            TENANT_ID,
            CUST_UBER_ID,
            "a step failed",
            now + chrono::Duration::minutes(10),
            now,
        )
        .await
        .expect("a failure");
    }
    assert!(controller(&env, CUST_UBER_ID).await.dirty_since.is_some());

    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 0, "not before its retry");
}

/// An input changing while the customer runs keeps it dirty, whatever the clocks: the run clears
/// only the mark it claimed.
#[rstest]
#[tokio::test]
async fn an_input_during_a_run_keeps_the_customer_dirty(#[future] test_env: TestEnv) {
    use diesel_models::billing_controller::BillingControllerRow;
    let env = test_env.await;
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let mut conn = env.conn().await;
    let now = common_clock::now();
    let claimed =
        BillingControllerRow::claim_due(&mut conn, now, now + chrono::Duration::minutes(5), 100)
            .await
            .expect("a claim")
            .into_iter()
            .find(|row| row.customer_id == CUST_UBER_ID)
            .expect("the customer");
    // Written by a replica whose clock lags: earlier than the claim, but a new mark.
    let lagging = chrono::SubsecRound::trunc_subsecs(now - chrono::Duration::seconds(30), 6);
    diesel::update(bc::table.filter(bc::customer_id.eq(CUST_UBER_ID)))
        .set(bc::dirty_since.eq(Some(lagging)))
        .execute(&mut conn)
        .await
        .expect("an input");
    BillingControllerRow::complete(
        &mut conn,
        TENANT_ID,
        CUST_UBER_ID,
        claimed.dirty_since,
        None,
        now,
    )
    .await
    .expect("the run ends");
    drop(conn);
    assert_eq!(
        controller(&env, CUST_UBER_ID).await.dirty_since,
        Some(lagging)
    );
}
