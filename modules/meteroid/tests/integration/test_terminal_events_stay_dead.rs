//! The previous release's scheduled events left failed, completed or cancelled were dead there
//! and stay so through the release's backfills: no ending, no deadline, nothing charged or issued.
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use common_domain::ids::{BaseId, SubscriptionId};
use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
use meteroid_store::domain::connectors::MockChargeBehavior;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

use crate::release_migrations::{
    BILLING_LEGACY_DROPPED_DOWN as DROPPED_DOWN, BILLING_ON_FACTS_UP as FACTS,
};

fn at(m: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, m, d, h, 0, 0).unwrap()
}

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

/// `sql` from the first `from` through the first `through` after it.
fn between<'a>(sql: &'a str, from: &str, through: &str) -> &'a str {
    let start = sql.find(from).expect("the section's start");
    let end = start + sql[start..].find(through).expect("the section's end") + through.len();
    &sql[start..end]
}

#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    n: i64,
}

async fn count(env: &TestEnv, sql: &str, sub_id: SubscriptionId) -> i64 {
    diesel::sql_query(sql)
        .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        .get_result::<Count>(&mut env.conn().await)
        .await
        .expect("a count")
        .n
}

/// Documents, notes, payment attempts, endings and deadlines of the subscription.
async fn state(env: &TestEnv, sub_id: SubscriptionId) -> [i64; 5] {
    [
        count(
            env,
            "SELECT count(*) AS n FROM invoice WHERE subscription_id = $1",
            sub_id,
        )
        .await,
        count(
            env,
            "SELECT count(*) AS n FROM credit_note n JOIN invoice i ON i.id = n.invoice_id
             WHERE i.subscription_id = $1",
            sub_id,
        )
        .await,
        count(
            env,
            "SELECT count(*) AS n FROM payment_transaction t JOIN invoice i ON i.id = t.invoice_id
             WHERE i.subscription_id = $1",
            sub_id,
        )
        .await,
        count(
            env,
            "SELECT count(*) AS n FROM subscription_ending WHERE subscription_id = $1",
            sub_id,
        )
        .await,
        count(
            env,
            "SELECT count(*) AS n FROM invoice WHERE subscription_id = $1
               AND (finalize_after IS NOT NULL OR next_payment_attempt_at IS NOT NULL)",
            sub_id,
        )
        .await,
    ]
}

/// A monthly subscription from Aug 1, its charges declined, renewed to November (that draft
/// waiting out a long grace), with main's dead events: a cancellation from Sep 1 that failed, one
/// completed and one cancelled; a retry of October's invoice that failed after its last attempt;
/// November's finalization failed; each also completed and cancelled. The release's backfills
/// make nothing of them, and a controller pass charges nothing and issues nothing.
/// Rules: R-016, R-077, R-080
#[rstest]
#[tokio::test]
async fn dead_events_of_the_previous_release_stay_dead(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_mock_charge_behavior(MockChargeBehavior::Failed)
        .await;
    env.set_consolidate_and_grace(false, 0).await;
    env.set_now(at(8, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(8, 1))
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    for month in [8, 9, 10] {
        env.set_now(at(month, 1, 1));
        env.process_cycles().await;
        env.run_outbox_and_orchestration().await;
    }
    env.set_consolidate_and_grace(false, 10_000).await;
    env.set_now(at(11, 1, 1));
    env.process_cycles().await;
    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    let of = |month: u32| {
        invoices
            .iter()
            .find(|invoice| invoice.billing_period_start == day(month, 1))
            .expect("the month's invoice")
            .id
            .as_uuid()
    };
    let (october, november) = (of(10), of(11));
    let mut conn = env.conn().await;
    // Main's state: dunning over, nothing pending on the invoices.
    conn.batch_execute(&format!(
        "UPDATE invoice SET finalize_after = NULL, next_payment_attempt_at = NULL
         WHERE subscription_id = '{}'",
        sub_id.as_uuid()
    ))
    .await
    .expect("main's invoices");
    conn.batch_execute(between(
        DROPPED_DOWN,
        "CREATE TYPE \"ScheduledEventTypeEnum\"",
        "WHERE status = 'PENDING' AND processed_at IS NULL;",
    ))
    .await
    .expect("main's scheduled_event");
    let mut events = Vec::new();
    for status in ["FAILED", "COMPLETED", "CANCELED"] {
        events.push(format!(
            "('CANCEL_SUBSCRIPTION', '2024-09-01', '{{\"CancelSubscription\": {{\"reason\": null}}}}', '{status}')"
        ));
        events.push(format!(
            "('RETRY_PAYMENT', '2024-11-05', '{{\"RetryPayment\": {{\"invoice_id\": \"{october}\"}}}}', '{status}')"
        ));
        events.push(format!(
            "('FINALIZE_INVOICE', '2024-11-02', '{{\"FinalizeInvoice\": {{\"invoice_id\": \"{november}\"}}}}', '{status}')"
        ));
    }
    conn.batch_execute(&format!(
        "INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time,
             priority, event_data, created_at, status, source)
         SELECT gen_random_uuid(), '{sub}', '{tenant}', e.kind::\"ScheduledEventTypeEnum\",
                e.at::timestamp, 0, e.data::jsonb, now(), e.status::\"ScheduledEventStatus\", 'api'
         FROM (VALUES {values}) AS e(kind, at, data, status)",
        sub = sub_id.as_uuid(),
        tenant = TENANT_ID.as_uuid(),
        values = events.join(", "),
    ))
    .await
    .expect("main's dead events");
    let before = state(&env, sub_id).await;

    conn.batch_execute(between(
        FACTS,
        "INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, actor_type,",
        "ORDER BY e.subscription_id, (e.status <> 'COMPLETED') DESC, e.created_at DESC;",
    ))
    .await
    .expect("the endings backfill");
    conn.batch_execute(between(
        FACTS,
        "UPDATE invoice i\nSET finalize_after",
        "AND i.payment_status <> 'PAID';",
    ))
    .await
    .expect("the deadlines backfill");
    conn.batch_execute(
        "DROP TABLE scheduled_event; DROP TYPE \"ScheduledEventTypeEnum\";
         DROP TYPE \"ScheduledEventStatus\";",
    )
    .await
    .expect("dropped");
    drop(conn);
    assert_eq!(
        state(&env, sub_id).await,
        before,
        "no ending and no deadline from a dead event"
    );
    assert_eq!(before[3..], [0, 0], "none before either");

    env.set_now(at(11, 15, 1));
    for _ in 0..10 {
        if env.run_billing_controller().await == 0 {
            break;
        }
    }
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        state(&env, sub_id).await,
        before,
        "nothing charged, issued or credited"
    );
}
