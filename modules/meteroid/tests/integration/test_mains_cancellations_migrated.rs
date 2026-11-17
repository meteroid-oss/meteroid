//! Main's cancellations, carried out or still to come, keep through the release's backfill the
//! instant they were requested, never the migration's: a controller pass after the release writes
//! nothing of them, and one still to come bills at its date as main would.
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
use diesel_models::enums::BillingPeriodEnum;
use meteroid_store::domain::BillingType;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use rstest::rstest;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
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

/// 500.00 a month billed in arrears.
async fn arrears_plan(env: &TestEnv) -> PlanVersionId {
    let version_id = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Arrears", version_id)
        .components(vec![SeedComp::extra_recurring(
            PriceComponentId::new(),
            "Arrears Fee",
            PRODUCT_RECURRING_ARREARS_ID,
            PriceId::new(),
            BillingPeriodEnum::Monthly,
            dec!(500),
            1,
            BillingType::Arrears,
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    version_id
}

/// Monthly in arrears from Aug 1, 2024, its invoices issued at once, renewed Sep 1, cancelled on
/// Sep 10 from Oct 1.
async fn cancelled_from_oct_1(env: &TestEnv) -> SubscriptionId {
    env.set_consolidate_and_grace(false, 0).await;
    env.set_now(at(8, 1, 1));
    let plan = arrears_plan(env).await;
    let sub_id = subscription()
        .plan_version(plan)
        .start_date(day(8, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(at(9, 1, 1));
    env.process_cycles().await;
    env.run_outbox_and_orchestration().await;
    env.set_now(at(9, 10, 10));
    env.services()
        .cancel_subscription(
            common_domain::actor::Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(10, 1)),
        )
        .await
        .expect("cancelled from Oct 1");
    sub_id
}

/// Each invoice's type, date, status and total, and the credit notes' count.
async fn issued(env: &TestEnv, sub_id: SubscriptionId) -> (Vec<String>, i64) {
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    let invoices = env
        .get_invoices(sub_id)
        .await
        .iter()
        .map(|invoice| {
            format!(
                "{:?} {} {:?} {}",
                invoice.invoice_type, invoice.invoice_date, invoice.status, invoice.total
            )
        })
        .collect();
    let notes = diesel::sql_query(
        "SELECT count(*) AS n FROM credit_note n JOIN invoice i ON i.id = n.invoice_id
         WHERE i.subscription_id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .get_result::<Count>(&mut env.conn().await)
    .await
    .expect("the notes")
    .n;
    (invoices, notes)
}

/// The cancellation as main left it, an event of `status` created at `requested`, no ending; then
/// the release's endings backfill, run after the ending's day, and scheduled_event dropped.
async fn as_migrated_from_main(
    env: &TestEnv,
    sub_id: SubscriptionId,
    status: &str,
    requested: NaiveDateTime,
) {
    let mut conn = env.conn().await;
    conn.batch_execute(between(
        DROPPED_DOWN,
        "CREATE TYPE \"ScheduledEventTypeEnum\"",
        "WHERE status = 'PENDING' AND processed_at IS NULL;",
    ))
    .await
    .expect("main's scheduled_event");
    conn.batch_execute(&format!(
        "DELETE FROM subscription_ending WHERE subscription_id = '{sub}' AND kind = 'CANCEL';
         INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time,
             priority, event_data, created_at, status, source)
         VALUES (gen_random_uuid(), '{sub}', '{tenant}', 'CANCEL_SUBSCRIPTION',
                 '2024-10-01 00:00:00', 0, '{{\"CancelSubscription\": {{\"reason\": null}}}}',
                 '{requested}', '{status}', 'api')",
        sub = sub_id.as_uuid(),
        tenant = TENANT_ID.as_uuid(),
    ))
    .await
    .expect("main's cancellation");
    conn.batch_execute(between(
        FACTS,
        "INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, actor_type,",
        "ORDER BY e.subscription_id, (e.status <> 'COMPLETED') DESC, e.created_at DESC;",
    ))
    .await
    .expect("the endings backfill");
    conn.batch_execute(
        "DROP TABLE scheduled_event; DROP TYPE \"ScheduledEventTypeEnum\";
         DROP TYPE \"ScheduledEventStatus\";",
    )
    .await
    .expect("dropped");
}

async fn requested_at(env: &TestEnv, sub_id: SubscriptionId) -> NaiveDateTime {
    #[derive(diesel::QueryableByName)]
    struct Requested {
        #[diesel(sql_type = diesel::sql_types::Timestamp)]
        requested_at: NaiveDateTime,
    }
    diesel::sql_query(
        "SELECT requested_at FROM subscription_ending WHERE subscription_id = $1 AND kind = 'CANCEL'",
    )
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .get_result::<Requested>(&mut env.conn().await)
    .await
    .expect("the ending")
    .requested_at
}

async fn controller_pass(env: &TestEnv) {
    for _ in 0..10 {
        if env.run_billing_controller().await == 0 {
            break;
        }
    }
    env.run_outbox_and_orchestration().await;
}

/// Ended on main on Oct 1, its September billed on the Oct 1 invoice and every invoice issued,
/// its cancellation requested on Sep 10, or on Oct 3 from Oct 1 as main allowed: migrated on
/// Nov 15, the ending keeps that request, and a controller pass writes no document.
/// Rules: R-016, R-102
#[rstest]
#[case::requested_in_advance(at(9, 10, 10))]
#[case::requested_after_its_day(at(10, 3, 10))]
#[tokio::test]
async fn a_cancellation_main_carried_out_writes_no_document_after_the_release(
    #[future] test_env: TestEnv,
    #[case] requested: DateTime<Utc>,
) {
    let env = test_env.await;
    let sub_id = cancelled_from_oct_1(&env).await;
    env.set_now(at(10, 1, 1));
    controller_pass(&env).await;
    let before = issued(&env, sub_id).await;
    assert_eq!(
        before,
        (
            vec![
                "Recurring 2024-09-01 Finalized 50000".to_string(),
                "Recurring 2024-10-01 Finalized 50000".to_string(),
            ],
            0
        ),
        "August and September, billed in arrears and issued"
    );

    as_migrated_from_main(&env, sub_id, "COMPLETED", requested.naive_utc()).await;
    env.set_now(at(11, 15, 1));
    controller_pass(&env).await;

    assert_eq!(issued(&env, sub_id).await, before, "no document written");
    assert_eq!(requested_at(&env, sub_id).await, requested.naive_utc());
}

/// Cancelled on main on Sep 10 from Oct 1, still to come when migrated on Sep 20: a controller
/// pass then writes no document, and on Oct 1 September bills 50000 on the recurring invoice of
/// that day, as main bills it, no final invoice.
/// Rules: R-016, R-011, R-102
#[rstest]
#[tokio::test]
async fn a_cancellation_main_left_to_come_bills_at_its_date_as_main_after_the_release(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let sub_id = cancelled_from_oct_1(&env).await;
    env.set_now(at(9, 20, 1));
    let before = issued(&env, sub_id).await;
    assert_eq!(
        before,
        (vec!["Recurring 2024-09-01 Finalized 50000".to_string()], 0)
    );

    as_migrated_from_main(&env, sub_id, "PENDING", at(9, 10, 10).naive_utc()).await;
    controller_pass(&env).await;
    assert_eq!(issued(&env, sub_id).await, before, "no document written");

    env.set_now(at(10, 1, 1));
    controller_pass(&env).await;
    assert_eq!(
        issued(&env, sub_id).await,
        (
            vec![
                "Recurring 2024-09-01 Finalized 50000".to_string(),
                "Recurring 2024-10-01 Finalized 50000".to_string(),
            ],
            0
        )
    );
    assert_eq!(requested_at(&env, sub_id).await, at(9, 10, 10).naive_utc());
}
