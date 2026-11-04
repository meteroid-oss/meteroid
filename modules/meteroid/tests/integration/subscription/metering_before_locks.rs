//! Work that locks a draft, its customer or its subscription reads metering before taking the
//! locks: while a read is held, the rows stay free, and an input that changes before the locks
//! are taken is billed as it stands then, never as it was read.
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, SubscriptionId};
use diesel_async::SimpleAsyncConnection;
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, IngestEventsRequest,
    IngestEventsResult, MutableMockUsageClient, UsageClient, UsageData, WindowedUsageData,
};
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{BillableMetric, Invoice, UsagePeriod};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

use crate::data::ids::{METRIC_BANDWIDTH, PLAN_VERSION_USAGE_ALPHA_ID, TENANT_ID};
use crate::harness::TestEnv;
use crate::helpers::credit::fund;

/// A metering store whose first read once armed answers, then holds its answer until released:
/// what it returns is what metering held when the read was made.
struct HeldUsage {
    inner: MutableMockUsageClient,
    armed: AtomicBool,
    entered: Notify,
    release: Notify,
}

impl HeldUsage {
    fn set(&self, units: i64) {
        let at = NaiveDateTime::default();
        self.inner.set_usage_for_any_nonempty_window(
            METRIC_BANDWIDTH,
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(units),
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod { start: at, end: at },
            },
        );
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }

    async fn held(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(60), self.entered.notified())
            .await
            .expect("the work reads metering");
    }
}

#[async_trait::async_trait]
impl UsageClient for HeldUsage {
    async fn fetch_usage(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<UsageData> {
        let read = self
            .inner
            .fetch_usage(tenant_id, customer_id, metric, period)
            .await;
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        read
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<UsageData> {
        self.inner
            .fetch_usage_grouped_by(tenant_id, customer_id, metric, group_by, period)
            .await
    }

    async fn fetch_total_usage(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<Decimal> {
        self.inner
            .fetch_total_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<WindowedUsageData> {
        self.inner
            .fetch_windowed_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_usage_summary(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<UsageData> {
        self.inner
            .fetch_usage_summary(tenant_id, customer_id, metric, period)
            .await
    }

    async fn search_events(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        options: EventSearchOptions,
    ) -> meteroid_store::StoreResult<EventSearchResult> {
        self.inner.search_events(tenant_id, options).await
    }

    async fn ingest_events(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        request: IngestEventsRequest,
    ) -> meteroid_store::StoreResult<IngestEventsResult> {
        self.inner.ingest_events(tenant_id, request).await
    }
}

fn held_usage() -> Arc<HeldUsage> {
    Arc::new(HeldUsage {
        inner: MutableMockUsageClient::new(),
        armed: AtomicBool::new(false),
        entered: Notify::new(),
        release: Notify::new(),
    })
}

/// Usage Alpha (€10 a month, API calls at €0.10) from January 2024, with 100 calls in any window.
async fn alpha(auto_advance: bool) -> (TestEnv, Arc<HeldUsage>, SubscriptionId, CustomerId) {
    let usage = held_usage();
    usage.set(100);
    let env = crate::harness::test_env_with_usage(usage.clone()).await;
    let builder = crate::harness::subscription()
        .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial();
    let builder = if auto_advance {
        builder
    } else {
        builder.no_auto_advance()
    };
    let subscription_id = builder.create(env.services()).await;
    let customer_id = env.get_subscription(subscription_id).await.customer_id;
    (env, usage, subscription_id, customer_id)
}

async fn invoice_of(env: &TestEnv, subscription_id: SubscriptionId, month: u32) -> Invoice {
    let start = NaiveDate::from_ymd_opt(2024, month, 1).unwrap();
    env.get_invoices(subscription_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == start)
        .expect("the month's invoice")
}

/// Takes and releases `lock` on the row at once, failing if anyone holds a conflicting lock.
async fn free(env: &TestEnv, table: &str, id: uuid::Uuid, lock: &str) {
    env.conn()
        .await
        .batch_execute(&format!(
            "BEGIN; SELECT id FROM {table} WHERE id = '{id}' {lock} NOWAIT; COMMIT;"
        ))
        .await
        .unwrap_or_else(|error| panic!("{table} {id} is held while metering is read: {error}"));
}

async fn draft_free(env: &TestEnv, invoice_id: InvoiceId, customer_id: CustomerId) {
    free(env, "invoice", invoice_id.as_uuid(), "FOR UPDATE").await;
    free(env, "customer", customer_id.as_uuid(), "FOR UPDATE").await;
}

/// The metric's definition changes while the read is held, and with it what metering counts.
async fn metric_redefined(env: &TestEnv, usage: &HeldUsage, units: i64) {
    usage.set(units);
    env.conn()
        .await
        .batch_execute(&format!(
            "UPDATE billable_metric SET updated_at = now() WHERE id = '{}'",
            METRIC_BANDWIDTH.as_uuid()
        ))
        .await
        .expect("the metric is redefined");
}

/// February's draft (January's 100 calls, €10, and February's €10 fee) is refreshed while its
/// usage read is held: the draft and its customer stay free, and January's draft finalizes
/// meanwhile, drawing €10 of the €25 of credit. The refresh then estimates on what is left (€15),
/// not on January's draft as it was before the read (€25 less its €10 estimate, then less again).
#[tokio::test]
async fn a_draft_refresh_reads_metering_without_holding_the_draft() {
    let (env, usage, subscription_id, customer_id) = alpha(false).await;
    env.process_cycle_transitions().await;
    let january = invoice_of(&env, subscription_id, 1).await;
    let february = invoice_of(&env, subscription_id, 2).await;
    assert_eq!(
        (january.status, february.status),
        (InvoiceStatusEnum::Draft, InvoiceStatusEnum::Draft)
    );
    assert_eq!(february.total, 2_000);
    fund(env.store(), TENANT_ID, customer_id, None, 2_500).await;

    usage.arm();
    let services = env.services().clone();
    let refreshing =
        tokio::spawn(async move { services.refresh_invoice_data(february.id, TENANT_ID).await });
    usage.held().await;
    draft_free(&env, february.id, customer_id).await;
    let january = env
        .services()
        .finalize_invoice(Actor::System, january.id, TENANT_ID)
        .await
        .expect("January finalizes while February's usage is read")
        .invoice;
    assert_eq!(january.applied_credits, 1_000);
    usage.release.notify_one();

    let refreshed = refreshing
        .await
        .expect("the refresh task")
        .expect("the refresh")
        .invoice;
    assert_eq!(refreshed.total, 2_000);
    assert_eq!(refreshed.applied_credits, 1_500);
    assert_eq!(refreshed.amount_due, 500);
}

/// February's draft finalizes while its usage read is held: the draft and its customer stay
/// free. The metric is redefined meanwhile (300 calls now): the finalization bills the 300 calls
/// metering counts once its locks are taken (€30 and the €10 fee), not the 100 it first read.
#[tokio::test]
async fn finalizing_reads_metering_without_holding_the_invoice() {
    let (env, usage, subscription_id, customer_id) = alpha(false).await;
    env.process_cycle_transitions().await;
    let january = invoice_of(&env, subscription_id, 1).await;
    env.services()
        .finalize_invoice(Actor::System, january.id, TENANT_ID)
        .await
        .expect("January");
    let february = invoice_of(&env, subscription_id, 2).await;

    usage.arm();
    let services = env.services().clone();
    let finalizing = tokio::spawn(async move {
        services
            .finalize_invoice(Actor::System, february.id, TENANT_ID)
            .await
    });
    usage.held().await;
    draft_free(&env, february.id, customer_id).await;
    metric_redefined(&env, &usage, 300).await;
    usage.release.notify_one();

    let finalized = finalizing
        .await
        .expect("the finalization task")
        .expect("the finalization")
        .invoice;
    assert_eq!(finalized.status, InvoiceStatusEnum::Finalized);
    assert_eq!(finalized.total, 4_000);
}

/// The renewal drafting February reads January's usage while the subscription stays free; the
/// metric redefined meanwhile, the draft bills the 300 calls counted once the subscription is
/// locked.
#[tokio::test]
async fn a_period_transition_reads_metering_without_holding_the_subscription() {
    let (env, usage, subscription_id, _) = alpha(true).await;
    env.set_consolidate_and_grace(false, 24).await;

    usage.arm();
    let services = env.services().clone();
    let renewing = tokio::spawn(async move { services.get_and_process_cycle_transitions().await });
    usage.held().await;
    free(
        &env,
        "subscription",
        subscription_id.as_uuid(),
        "FOR UPDATE",
    )
    .await;
    metric_redefined(&env, &usage, 300).await;
    usage.release.notify_one();
    renewing
        .await
        .expect("the renewal task")
        .expect("the renewal");

    let february = invoice_of(&env, subscription_id, 2).await;
    assert_eq!(february.status, InvoiceStatusEnum::Draft);
    assert_eq!(february.total, 4_000);
    let subscription = env.get_subscription(subscription_id).await;
    assert_eq!(subscription.error_count, 0);
}

/// The grace period's scheduled finalization reads February's usage while the draft and its
/// customer stay free, and bills the 300 calls counted once they are locked.
#[tokio::test]
async fn a_scheduled_finalization_reads_metering_without_holding_the_draft() {
    let (env, usage, subscription_id, customer_id) = alpha(true).await;
    env.set_consolidate_and_grace(false, 24).await;
    env.process_cycle_transitions().await;
    let february = invoice_of(&env, subscription_id, 2).await;
    assert_eq!(february.status, InvoiceStatusEnum::Draft);
    assert_eq!(february.total, 2_000);

    usage.arm();
    let services = env.services().clone();
    let finalizing = tokio::spawn(async move { services.get_and_process_due_events().await });
    usage.held().await;
    draft_free(&env, february.id, customer_id).await;
    metric_redefined(&env, &usage, 300).await;
    usage.release.notify_one();
    finalizing
        .await
        .expect("the scheduled events task")
        .expect("the scheduled events");

    let finalized = invoice_of(&env, subscription_id, 2).await;
    assert_eq!(finalized.status, InvoiceStatusEnum::Finalized);
    assert_eq!(finalized.total, 4_000);
}

/// The renewal's locked pass finds the customer taken by another transaction while its usage was
/// read: the worker defers the subscription for a short while without an error, and the first
/// tick after that renews it.
#[tokio::test]
async fn a_customer_taken_while_the_renewal_reads_metering_waits_for_the_next_tick() {
    let (env, usage, subscription_id, customer_id) = alpha(true).await;
    env.set_consolidate_and_grace(false, 24).await;
    let invoiced = env.get_invoices(subscription_id).await.len();

    usage.arm();
    let services = env.services().clone();
    let renewing = tokio::spawn(async move { services.get_and_process_cycle_transitions().await });
    usage.held().await;
    free(&env, "customer", customer_id.as_uuid(), "FOR UPDATE").await;
    let mut holder = env.conn().await;
    holder
        .batch_execute(&format!(
            "BEGIN; SELECT id FROM customer WHERE id = '{}' FOR UPDATE;",
            customer_id.as_uuid()
        ))
        .await
        .expect("another transaction takes the customer");
    usage.release.notify_one();
    let skipped = renewing
        .await
        .expect("the renewal task")
        .expect("a busy customer is not a failure");
    assert_eq!((skipped.processed, skipped.has_more), (0, false));
    let deferred = env.get_subscription(subscription_id).await;
    assert_eq!(deferred.error_count, 0);
    assert!(
        deferred
            .next_retry
            .is_some_and(|at| at > chrono::Utc::now().naive_utc() + chrono::Duration::seconds(20)),
        "deferred: {:?}",
        deferred.next_retry
    );
    assert_eq!(env.get_invoices(subscription_id).await.len(), invoiced);
    holder
        .batch_execute("COMMIT;")
        .await
        .expect("the customer is given back");
    let deferral_over = env
        .services()
        .get_and_process_cycle_transitions()
        .await
        .expect("a tick within the deferral");
    assert_eq!(
        deferral_over.processed, 0,
        "deferred, not claimed again at once"
    );
    env.conn()
        .await
        .batch_execute(&format!(
            "UPDATE subscription SET next_retry = now() - interval '1 second' WHERE id = '{}'",
            subscription_id.as_uuid()
        ))
        .await
        .expect("the deferral is over");

    let renewed = env
        .services()
        .get_and_process_cycle_transitions()
        .await
        .expect("the next tick");
    assert_eq!(renewed.processed, 1);
    let february = invoice_of(&env, subscription_id, 2).await;
    assert_eq!(february.status, InvoiceStatusEnum::Draft);
    assert_eq!(
        february.total, 2_000,
        "January's 100 calls at 0.10 and February's 10.00 fee"
    );
    assert_eq!(env.get_subscription(subscription_id).await.error_count, 0);
}

/// Scale -> Studio on Feb 15 while February's draft waits out its grace; the draft is issued
/// without the change's settlement (an earlier release, a missed hook). The sweep catching it up
/// reads the removed pack's usage while the customer stays free.
#[tokio::test]
async fn the_missed_ending_sweep_reads_metering_without_holding_the_customer() {
    use super::credit_plan_change::{
        SCALE, STUDIO, change_on, date, halyard, invoice, pay, scale_usage, start,
    };
    use crate::data::ids::CUST_UBER_ID;
    use diesel_async::RunQueryDsl;
    use meteroid_store::domain::price_components::CreditExpiry;

    let usage = held_usage();
    usage.set(1_000);
    let env = crate::harness::test_env_with_usage(usage.clone()).await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    env.process_cycle_transitions().await;
    change_on(&env, sub_id, STUDIO, date(2024, 2, 15)).await;

    let flag = |sql: &'static str| {
        let env = &env;
        async move {
            let mut conn = env.conn().await;
            diesel::sql_query(sql)
                .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
                .execute(&mut conn)
                .await
                .expect("switch event");
        }
    };
    flag("UPDATE subscription_event SET details = details - 'outgoing' WHERE subscription_id = $1")
        .await;
    env.process_due_events().await;
    flag(
        "UPDATE subscription_event SET details = details || '{\"outgoing\": \"settled\"}' \
         WHERE subscription_id = $1 AND details ? 'new_plan_version_id'",
    )
    .await;

    usage.arm();
    let services = env.services().clone();
    let sweeping =
        tokio::spawn(async move { services.settle_missed_plan_change_endings(100).await });
    usage.held().await;
    free(&env, "customer", CUST_UBER_ID.as_uuid(), "FOR UPDATE").await;
    usage.release.notify_one();
    let issued = sweeping.await.expect("the sweep task").expect("the sweep");
    assert!(issued >= 1, "the change is settled on the issued invoice");
    assert_eq!(
        env.services()
            .settle_missed_plan_change_endings(100)
            .await
            .expect("the next sweep"),
        0
    );
}

/// Fails if a session of this test's database has a transaction open.
async fn no_transaction_open(env: &TestEnv) {
    use diesel_async::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Open {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    let open: Open = diesel::sql_query(
        "SELECT count(*) AS n FROM pg_stat_activity \
         WHERE datname = current_database() AND pid <> pg_backend_pid() \
         AND xact_start IS NOT NULL",
    )
    .get_result(&mut env.conn().await)
    .await
    .expect("sessions");
    assert_eq!(open.n, 0, "a transaction is open while metering is read");
}

/// Holds each metering read `work` makes and runs `check` while it is held.
async fn each_read<T, F>(usage: &HeldUsage, mut work: tokio::task::JoinHandle<T>, check: F) -> T
where
    F: AsyncFn(),
{
    let mut reads = 0;
    loop {
        usage.arm();
        tokio::select! {
            _ = usage.entered.notified() => {
                reads += 1;
                check().await;
                usage.release.notify_one();
            }
            done = &mut work => {
                usage.disarm();
                assert!(reads > 0, "the work reads metering");
                return done.expect("the work's task");
            }
        }
    }
}

/// An immediate Scale -> Studio change ends Scale's period pack ten days into the period. The
/// preview's estimate of the pack and the plan-change checkout's amount settle the pack's usage in
/// a transaction rolled back: they read that usage outside any transaction, with the customer
/// free, and price exactly what they price when metering answers at once.
#[tokio::test]
async fn plan_change_previews_read_metering_outside_transactions() {
    use super::credit_plan_change::{SCALE, STUDIO, halyard, invoice, pay, scale_usage, start};
    use crate::data::ids::CUST_UBER_ID;
    use meteroid_store::domain::price_components::CreditExpiry;
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let usage = held_usage();
    usage.set(1_000);
    let env = crate::harness::test_env_with_usage(usage.clone()).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let today = chrono::Utc::now().date_naive();
    let sub_id = start(&env, CUST_UBER_ID, SCALE, today - chrono::Days::new(10)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;

    let preview = |services: meteroid_store::Services| async move {
        services
            .preview_plan_change(
                sub_id,
                TENANT_ID,
                STUDIO,
                vec![],
                Some(PlanChangeMode::Immediate),
            )
            .await
            .expect("the preview")
            .proration
            .map(|proration| serde_json::to_value(proration).unwrap())
    };
    let services = env.services().clone();
    let held = each_read(&usage, tokio::spawn(preview(services)), || async {
        no_transaction_open(&env).await;
    })
    .await;
    assert_eq!(held, preview(env.services().clone()).await);

    let checkout = |services: meteroid_store::Services| async move {
        let computed = services
            .compute_plan_change_checkout_invoice(sub_id, TENANT_ID, STUDIO, today)
            .await
            .expect("the checkout amount");
        (
            computed.subtotal,
            computed.applied_credits,
            computed.tax_amount,
            computed.total,
            computed.amount_due,
        )
    };
    let services = env.services().clone();
    let held = each_read(&usage, tokio::spawn(checkout(services)), || async {
        no_transaction_open(&env).await;
        free(&env, "customer", CUST_UBER_ID.as_uuid(), "FOR UPDATE").await;
    })
    .await;
    assert_eq!(held, checkout(env.services().clone()).await);
}

/// The portal checkout applying an immediate Scale -> Studio change reads the old pack's usage
/// before its transaction locks the customer; under the lock it is served those reads and never
/// reads metering. The change applies as quoted.
#[tokio::test]
async fn a_plan_change_checkout_reads_metering_before_locking_the_customer() {
    use super::credit_plan_change::{SCALE, STUDIO, halyard, invoice, pay, scale_usage, start};
    use crate::data::ids::{CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID};
    use meteroid_store::domain::checkout_sessions::CheckoutCompletionResult;
    use meteroid_store::domain::price_components::CreditExpiry;

    let usage = held_usage();
    usage.set(1_000);
    let env = crate::harness::test_env_with_usage(usage.clone()).await;
    env.seed_payments().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let today = chrono::Utc::now().date_naive();
    let sub_id = start(&env, CUST_UBER_ID, SCALE, today - chrono::Days::new(10)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let quote = env
        .services()
        .compute_plan_change_checkout_invoice(sub_id, TENANT_ID, STUDIO, today)
        .await
        .expect("the quote");
    assert!(quote.amount_due >= 0, "{}", quote.amount_due);
    let session = env
        .services()
        .create_plan_change_checkout_session(TENANT_ID, sub_id, STUDIO, CUST_UBER_ID, None, today)
        .await
        .expect("the session");

    let services = env.services().clone();
    let completing = tokio::spawn(async move {
        services
            .complete_checkout(
                TENANT_ID,
                session.id,
                CUST_UBER_PAYMENT_METHOD_ID,
                quote.amount_due as u64,
                "EUR".to_owned(),
                None,
            )
            .await
    });
    let completed = each_read(&usage, completing, || async {
        no_transaction_open(&env).await;
        free(&env, "customer", CUST_UBER_ID.as_uuid(), "FOR UPDATE").await;
    })
    .await
    .expect("the checkout");
    assert!(
        matches!(completed, CheckoutCompletionResult::Completed { .. }),
        "{completed:?}"
    );
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, STUDIO);
}
