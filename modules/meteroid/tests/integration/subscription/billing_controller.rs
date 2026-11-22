//! The billing controller: when a customer runs, what a run takes, and the sweep that finds a
//! lost wake.
use chrono::{NaiveDate, TimeZone, Utc};
use common_domain::ids::CustomerId;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::schema::billing_controller as bc;
use meteroid_store::domain::SubscriptionActivationCondition;
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

/// A controller three periods late writes every missed period's invoice in one run, each on its
/// period's start, and the subscription is current after it; it then wakes at the next boundary.
#[rstest]
#[tokio::test]
async fn a_late_controller_writes_every_missed_period_in_one_run(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    let sub = subscription()
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
    env.set_now(Utc.with_ymd_and_hms(2024, 4, 15, 12, 0, 0).unwrap());

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
    let current = env.get_subscription(sub).await;
    assert_eq!(current.current_period_start, day(2024, 4, 1));
    assert_eq!(current.current_period_end, Some(day(2024, 5, 1)));
    let invoices = env.get_invoices(sub).await;
    let mut periods: Vec<(NaiveDate, i64)> = invoices
        .iter()
        .map(|invoice| (invoice.billing_period_start, invoice.subtotal))
        .collect();
    periods.sort();
    // LeetCode's 35.00 a month in advance, one invoice per period.
    assert_eq!(
        periods,
        vec![
            (day(2024, 1, 1), 3500),
            (day(2024, 2, 1), 3500),
            (day(2024, 3, 1), 3500),
            (day(2024, 4, 1), 3500),
        ]
    );
    // It took work: it runs again at once, until nothing is left before the next boundary.
    let wake = controller(&env, CUST_UBER_ID).await.next_wake_at;
    assert!(wake.is_some_and(|at| at <= common_clock::now()));
    for _ in 0..5 {
        if controller(&env, CUST_UBER_ID)
            .await
            .next_wake_at
            .is_some_and(|at| at > common_clock::now())
        {
            break;
        }
        env.services()
            .run_billing_controller(100)
            .await
            .expect("a run");
    }
    assert_eq!(
        env.get_invoices(sub).await.len(),
        4,
        "nothing written twice"
    );
    assert_eq!(
        controller(&env, CUST_UBER_ID).await.next_wake_at,
        Some(Utc.with_ymd_and_hms(2024, 5, 1, 0, 0, 0).unwrap())
    );
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

/// LeetCode from Jan 1, created on Jan 15 with a card on file, awaiting `activation`.
async fn awaiting_its_start_from_jan_1(
    env: &TestEnv,
    activation: SubscriptionActivationCondition,
) -> common_domain::ids::SubscriptionId {
    env.seed_payments().await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .activation(activation)
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await
}

/// A subscription past its start awaiting its checkout (or its manual activation) has no wake:
/// only that moves it, and it marks the customer. Its customer is not rechecked every minute,
/// and the sweep counts no lost wake of it, a period later either.
#[rstest]
#[case::checkout(SubscriptionActivationCondition::OnCheckout, true)]
#[case::manual(SubscriptionActivationCondition::Manual, false)]
#[tokio::test]
async fn a_subscription_awaiting_its_checkout_has_no_wake(
    #[future] test_env: TestEnv,
    #[case] activation: SubscriptionActivationCondition,
    #[case] pending_checkout: bool,
) {
    let env = test_env.await;
    let sub = awaiting_its_start_from_jan_1(&env, activation).await;
    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 1, "the new subscription is an input");
    assert_eq!(env.get_subscription(sub).await.next_wake_at, None);
    let after = controller(&env, CUST_UBER_ID).await;
    assert_eq!(after.next_wake_at, None, "nothing to wake for");
    assert_eq!(after.dirty_since, None);

    env.advance(chrono::Duration::minutes(2));
    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 0, "not rechecked every minute");

    env.set_now(Utc.with_ymd_and_hms(2024, 2, 15, 6, 0, 0).unwrap());
    let sweep = env
        .services()
        .sweep_billing_controller()
        .await
        .expect("the sweep");
    assert_eq!(sweep.lost, 0, "no wake lost");
    assert_eq!(controller(&env, CUST_UBER_ID).await.next_wake_at, None);
    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 0);
    let pending = env.get_subscription(sub).await;
    assert_eq!(pending.status, SubscriptionStatusEnum::PendingActivation);
    assert_eq!(pending.pending_checkout, pending_checkout);
    assert_eq!(pending.next_wake_at, None);
    assert!(
        env.get_invoices(sub).await.is_empty(),
        "nothing billed before its start settles"
    );
}

/// A wake stored at the start of a subscription awaiting its checkout, its customer rechecked
/// every minute, is cleared by the next run: the sweep then counts no lost wake.
#[rstest]
#[tokio::test]
async fn a_stale_wake_awaiting_the_checkout_is_cleared_by_the_next_run(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub =
        awaiting_its_start_from_jan_1(&env, SubscriptionActivationCondition::OnCheckout).await;
    env.services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    {
        let mut conn = env.conn().await;
        diesel::sql_query(
            "UPDATE subscription SET next_wake_at = '2024-01-01 00:00:00+00' WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub)
        .execute(&mut conn)
        .await
        .expect("a stale wake");
        diesel::update(bc::table.filter(bc::customer_id.eq(CUST_UBER_ID)))
            .set(bc::next_wake_at.eq(Some(common_clock::now() + chrono::Duration::minutes(1))))
            .execute(&mut conn)
            .await
            .expect("a recheck");
    }

    env.advance(chrono::Duration::minutes(2));
    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 1);
    assert_eq!(env.get_subscription(sub).await.next_wake_at, None);
    assert_eq!(controller(&env, CUST_UBER_ID).await.next_wake_at, None);
    let sweep = env
        .services()
        .sweep_billing_controller()
        .await
        .expect("the sweep");
    assert_eq!(sweep.lost, 0);
}

/// An On Start subscription past its start, still pending with no wake (never evaluated), wakes
/// its customer at once, and the run activates it: only a checkout or an activation leaves none.
#[rstest]
#[tokio::test]
async fn an_on_start_subscription_with_no_wake_past_its_start_is_activated(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    let sub = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 2, 1))
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
        diesel::sql_query(
            "UPDATE subscription SET next_wake_at = NULL, current_period_end = NULL WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub)
        .execute(&mut conn)
        .await
        .expect("no wake");
        diesel::update(bc::table.filter(bc::customer_id.eq(CUST_UBER_ID)))
            .set((
                bc::next_wake_at.eq(None::<chrono::DateTime<Utc>>),
                bc::dirty_since.eq(None::<chrono::DateTime<Utc>>),
            ))
            .execute(&mut conn)
            .await
            .expect("no customer wake");
    }
    env.set_now(Utc.with_ymd_and_hms(2024, 2, 15, 6, 0, 0).unwrap());
    let pending = env.get_subscription(sub).await;
    assert_eq!(pending.status, SubscriptionStatusEnum::PendingActivation);
    assert_eq!(pending.next_wake_at, None);

    env.services()
        .sweep_billing_controller()
        .await
        .expect("the sweep");
    assert!(
        controller(&env, CUST_UBER_ID)
            .await
            .next_wake_at
            .is_some_and(|at| at <= common_clock::now()),
        "due at once"
    );
    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 1);
    let active = env.get_subscription(sub).await;
    assert_eq!(active.status, SubscriptionStatusEnum::Active);
    assert_eq!(active.current_period_start, day(2024, 2, 1));
    assert_eq!(active.current_period_end, Some(day(2024, 3, 1)));
}

/// Completing the checkout marks the customer: its run activates the subscription and wakes the
/// customer at the renewal, as any active subscription's.
#[rstest]
#[tokio::test]
async fn a_completed_checkout_wakes_the_customer_at_its_renewal(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sub =
        awaiting_its_start_from_jan_1(&env, SubscriptionActivationCondition::OnCheckout).await;
    env.services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(controller(&env, CUST_UBER_ID).await.next_wake_at, None);

    env.advance(chrono::Duration::hours(1));
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout");
    drop(conn);
    assert!(
        controller(&env, CUST_UBER_ID).await.dirty_since.is_some(),
        "the checkout is an input"
    );

    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 1);
    for _ in 0..5 {
        if controller(&env, CUST_UBER_ID)
            .await
            .next_wake_at
            .is_none_or(|at| at > common_clock::now())
        {
            break;
        }
        env.services()
            .run_billing_controller(100)
            .await
            .expect("a run");
    }
    let active = env.get_subscription(sub).await;
    assert_eq!(active.status, SubscriptionStatusEnum::Active);
    assert!(!active.pending_checkout);
    assert_eq!(active.current_period_end, Some(day(2024, 2, 1)));
    let renewal = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();
    assert_eq!(active.next_wake_at, Some(renewal));
    assert_eq!(
        controller(&env, CUST_UBER_ID).await.next_wake_at,
        Some(renewal)
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
        use diesel_models::billing_controller::BillingControllerRow;
        let mut conn = env.conn().await;
        let now = common_clock::now();
        let claimed = BillingControllerRow::claim_due(
            &mut conn,
            now,
            now + chrono::Duration::minutes(5),
            100,
        )
        .await
        .expect("a claim")
        .into_iter()
        .find(|row| row.customer_id == CUST_UBER_ID)
        .expect("the customer");
        // An input after the claim, which the failure keeps.
        diesel::update(bc::table.filter(bc::customer_id.eq(CUST_UBER_ID)))
            .set(bc::dirty_since.eq(Some(now)))
            .execute(&mut conn)
            .await
            .expect("an input");
        BillingControllerRow::fail(
            &mut conn,
            TENANT_ID,
            CUST_UBER_ID,
            claimed.lease_until.expect("leased"),
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
        claimed.lease_until.expect("leased"),
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

/// A run that outlived its lease ends after the replica that took the customer over: the stale
/// run's end writes nothing, the second owner's wake stays. Each run renews its lease when it
/// starts the customer, so a customer late in a claimed batch never runs past its lease.
#[rstest]
#[tokio::test]
async fn two_runs_with_an_expired_lease_write_one_wake(#[future] test_env: TestEnv) {
    use diesel_models::billing_controller::BillingControllerRow;
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap());
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let mut conn = env.conn().await;
    let first_at = common_clock::now();
    let first = BillingControllerRow::claim_due(
        &mut conn,
        first_at,
        first_at + chrono::Duration::minutes(5),
        100,
    )
    .await
    .expect("a claim")
    .into_iter()
    .find(|row| row.customer_id == CUST_UBER_ID)
    .expect("the customer");
    env.advance(chrono::Duration::minutes(6));
    let second_at = common_clock::now();
    let second = BillingControllerRow::claim_due(
        &mut conn,
        second_at,
        second_at + chrono::Duration::minutes(5),
        100,
    )
    .await
    .expect("a claim")
    .into_iter()
    .find(|row| row.customer_id == CUST_UBER_ID)
    .expect("taken over past the lease");
    let kept = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();
    let stale = Utc.with_ymd_and_hms(2024, 1, 1, 12, 7, 0).unwrap();
    let ended = BillingControllerRow::complete(
        &mut conn,
        TENANT_ID,
        CUST_UBER_ID,
        second.lease_until.expect("leased"),
        second.dirty_since,
        Some(kept),
        second_at,
    )
    .await
    .expect("the second run ends");
    assert!(ended);
    let ended = BillingControllerRow::complete(
        &mut conn,
        TENANT_ID,
        CUST_UBER_ID,
        first.lease_until.expect("leased"),
        first.dirty_since,
        Some(stale),
        second_at,
    )
    .await
    .expect("the stale run ends");
    assert!(!ended, "the stale run no longer holds the customer");
    let failed = BillingControllerRow::fail(
        &mut conn,
        TENANT_ID,
        CUST_UBER_ID,
        first.lease_until.expect("leased"),
        "stale",
        stale,
        second_at,
    )
    .await
    .expect("the stale failure");
    assert!(!failed);
    drop(conn);
    assert_eq!(
        controller(&env, CUST_UBER_ID).await.next_wake_at,
        Some(kept)
    );
}

/// A payment attempt is taken once: a second run seeing it due before the first committed takes
/// nothing, so one charge is requested.
#[rstest]
#[tokio::test]
async fn a_payment_attempt_is_taken_once(#[future] test_env: TestEnv) {
    use diesel_models::invoices::InvoiceRow;
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let invoice = env.get_invoices(sub_id).await.remove(0);
    let due = Utc
        .with_ymd_and_hms(2024, 1, 1, 12, 0, 0)
        .unwrap()
        .naive_utc();
    let mut conn = env.conn().await;
    InvoiceRow::set_next_payment_attempt_at(&mut conn, TENANT_ID, invoice.id, Some(due))
        .await
        .expect("an attempt due");
    let now = common_clock::now().naive_utc();
    let taken = [
        InvoiceRow::take_due_payment_attempt(&mut conn, TENANT_ID, invoice.id, now)
            .await
            .expect("taken"),
        InvoiceRow::take_due_payment_attempt(&mut conn, TENANT_ID, invoice.id, now)
            .await
            .expect("taken"),
    ];
    assert_eq!(taken, [true, false]);
}

/// A deadline written outside a run while the customer runs marks it dirty, so the run's end,
/// which read the wakes before it, never loses that deadline.
#[rstest]
#[tokio::test]
async fn a_deadline_written_during_a_run_keeps_the_customer_dirty(#[future] test_env: TestEnv) {
    use diesel_models::billing_controller::BillingControllerRow;
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap());
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
    env.advance(chrono::Duration::seconds(1));
    let deadline = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
    BillingControllerRow::wake_by(&mut conn, TENANT_ID, CUST_UBER_ID, deadline)
        .await
        .expect("a deadline");
    BillingControllerRow::complete(
        &mut conn,
        TENANT_ID,
        CUST_UBER_ID,
        claimed.lease_until.expect("leased"),
        claimed.dirty_since,
        Some(Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap()),
        common_clock::now(),
    )
    .await
    .expect("the run ends");
    drop(conn);
    assert!(
        controller(&env, CUST_UBER_ID).await.dirty_since.is_some(),
        "runs again, reading the deadline"
    );
}

async fn set_tenant_status(env: &TestEnv, status: &str) {
    use common_domain::ids::BaseId;
    diesel::sql_query(format!(
        "UPDATE tenant SET status = '{status}' WHERE id = '{}'",
        TENANT_ID.as_uuid()
    ))
    .execute(&mut env.conn().await)
    .await
    .expect("tenant status");
}

/// A tenant paused across a renewal and the sweep, then reactivated: its customer is claimed at
/// once and the renewal written, without waiting for the next sweep.
#[rstest]
#[tokio::test]
async fn a_reactivated_tenant_is_claimed_at_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_billing_controller().await;

    set_tenant_status(&env, "PAUSED").await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap());
    env.services()
        .sweep_billing_controller()
        .await
        .expect("the sweep while paused");
    env.set_now(Utc.with_ymd_and_hms(2024, 2, 2, 10, 0, 0).unwrap());
    set_tenant_status(&env, "ACTIVE").await;
    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a run");
    assert_eq!(run.customers, 1, "claimed without a sweep");
    assert!(
        env.get_invoices(sub_id)
            .await
            .iter()
            .any(|invoice| invoice.billing_period_start == day(2024, 2, 1)),
        "February's renewal is written"
    );
}

/// A subscription whose renewals write no document (a free plan billing nothing) catches up in
/// one run, each renewal a step that took work: the customer then wakes at its next renewal, never
/// a minute later to step one more period, which the sweep counted as a lost wake.
/// Rules: R-104
#[rstest]
#[tokio::test]
async fn a_free_plan_catching_up_wakes_at_its_next_renewal(#[future] test_env: TestEnv) {
    use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
    use diesel_models::enums::BillingPeriodEnum;
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    let version = PlanVersionId::new();
    crate::data::plans::PlanSeed::new(PlanId::new(), "Free at nothing", version)
        .free()
        .components(vec![crate::data::plans::SeedComp::rate(
            PriceComponentId::new(),
            "Free Rate",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            BillingPeriodEnum::Monthly,
            rust_decimal::Decimal::ZERO,
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a free plan");
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_billing_controller().await;

    env.set_now(Utc.with_ymd_and_hms(2024, 4, 10, 9, 0, 0).unwrap());
    // The run that took work runs the customer again at once, which finds nothing due.
    for expected in [1, 1, 0] {
        let run = env
            .services()
            .run_billing_controller(100)
            .await
            .expect("a run");
        assert_eq!(run.customers, expected);
    }
    let renewed = env.get_subscription(sub_id).await;
    assert_eq!(
        (renewed.current_period_start, renewed.current_period_end),
        (day(2024, 4, 1), Some(day(2024, 5, 1)))
    );
    assert_eq!(
        controller(&env, CUST_UBER_ID).await.next_wake_at,
        Some(Utc.with_ymd_and_hms(2024, 5, 1, 0, 0, 0).unwrap())
    );
    let swept = env
        .services()
        .sweep_billing_controller()
        .await
        .expect("the sweep");
    assert_eq!(swept.lost, 0);
}

async fn failures(env: &TestEnv) -> (Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>) {
    bc::table
        .filter(bc::customer_id.eq(CUST_UBER_ID))
        .select((bc::first_failed_at, bc::last_failed_at))
        .first(&mut env.conn().await)
        .await
        .expect("the row")
}

/// A failed run records when the failures in a row started and when the latest one was; the run
/// that completes clears both.
#[rstest]
#[tokio::test]
async fn a_failed_run_records_its_first_and_latest_failure(#[future] test_env: TestEnv) {
    use diesel_models::billing_controller::BillingControllerRow;
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap());
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let mut times = Vec::new();
    for _ in 0..2 {
        let mut conn = env.conn().await;
        let now = chrono::SubsecRound::trunc_subsecs(common_clock::now(), 6);
        let claimed = BillingControllerRow::claim_due(
            &mut conn,
            now,
            now + chrono::Duration::minutes(5),
            100,
        )
        .await
        .expect("a claim")
        .into_iter()
        .find(|row| row.customer_id == CUST_UBER_ID)
        .expect("the customer");
        BillingControllerRow::fail(
            &mut conn,
            TENANT_ID,
            CUST_UBER_ID,
            claimed.lease_until.expect("leased"),
            "a step failed",
            now,
            now,
        )
        .await
        .expect("a failure");
        times.push(now);
        env.advance(chrono::Duration::minutes(10));
    }
    assert_eq!(failures(&env).await, (Some(times[0]), Some(times[1])));
    env.run_billing_controller().await;
    assert_eq!(failures(&env).await, (None, None));
}

/// A draft whose finalization fails at every run never holds back the customer's credit: a grant
/// past its lapse still lapses in the same run.
#[rstest]
#[tokio::test]
async fn a_failing_finalize_does_not_stop_a_credit_lapse(#[future] test_env: TestEnv) {
    use diesel_models::schema::customer_credit_grant as g;
    use meteroid_testkit::helpers::credit::{Grant, expire_grant, fund_grant};
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_billing_controller().await;
    env.set_now(Utc.with_ymd_and_hms(2024, 2, 1, 2, 0, 0).unwrap());
    env.process_cycle_transitions().await;
    let february = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(2024, 2, 1))
        .expect("February's draft");
    // A draft due for its finalization, which the database refuses.
    let id = common_domain::ids::BaseId::as_uuid(&february.id);
    for sql in [
        format!("UPDATE invoice SET finalize_after = '2024-01-31 00:00:00' WHERE id = '{id}'"),
        "CREATE FUNCTION refuse_finalizing() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'finalizing refused'; END $$"
            .to_string(),
        format!(
            "CREATE TRIGGER refuse_finalizing BEFORE UPDATE ON invoice FOR EACH ROW
             WHEN (OLD.id = '{id}' AND NEW.status <> 'DRAFT') EXECUTE FUNCTION refuse_finalizing()"
        ),
    ] {
        diesel::sql_query(sql)
            .execute(&mut env.conn().await)
            .await
            .expect("a finalization refused");
    }
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        Grant::invoice(1_000),
    )
    .await;
    expire_grant(env.store(), grant).await;
    // Past the entity's settlement grace.
    env.advance(chrono::Duration::days(2));

    env.run_billing_controller().await;
    let remaining: i64 = g::table
        .filter(g::id.eq(grant))
        .select(g::remaining)
        .first(&mut env.conn().await)
        .await
        .expect("the grant");
    assert_eq!(remaining, 0, "lapsed");
}

/// One subscription whose cadence origin cannot be decided never stops the daily sweep: every
/// other customer is rewoken and its lost wake counted.
#[rstest]
#[tokio::test]
async fn a_failing_origin_does_not_stop_the_sweep(#[future] test_env: TestEnv) {
    use common_domain::ids::BaseId;
    let env = test_env.await;
    env.set_now(Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap());
    let undecidable = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .customer(CUST_COMODO_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2024, 1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.run_billing_controller().await;
    for sql in [
        format!(
            "INSERT INTO subscription_plan_window_origin_pending (window_id)
             SELECT id FROM subscription_plan_window WHERE subscription_id = '{}'",
            undecidable.as_uuid()
        ),
        "CREATE FUNCTION refuse_deciding() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'origin undecidable'; END $$"
            .to_string(),
        "CREATE TRIGGER refuse_deciding BEFORE DELETE ON subscription_plan_window_origin_pending
         FOR EACH ROW EXECUTE FUNCTION refuse_deciding()"
            .to_string(),
        // Uber's renewal is due at Feb 1, its stored wake lost.
        format!(
            "UPDATE billing_controller SET next_wake_at = '2030-01-01 00:00:00+00', dirty_since = NULL
             WHERE customer_id = '{}'",
            CUST_UBER_ID.as_uuid()
        ),
    ] {
        diesel::sql_query(sql)
            .execute(&mut env.conn().await)
            .await
            .expect("setup");
    }
    env.set_now(Utc.with_ymd_and_hms(2024, 2, 1, 12, 0, 0).unwrap());
    let swept = env
        .services()
        .sweep_billing_controller()
        .await
        .expect("the sweep goes on");
    assert_eq!(swept.lost, 1);
    assert_eq!(swept.pending_origins, 1);
    assert!(
        controller(&env, CUST_UBER_ID)
            .await
            .next_wake_at
            .is_some_and(|at| at <= Utc.with_ymd_and_hms(2024, 2, 1, 12, 0, 0).unwrap()),
        "rewoken"
    );
}
