//! Row locks never form a cycle with the customer lock, and metering failing between the passes
//! of locked work is the customer's failed run, recorded on its controller row like any failure.
use chrono::{Duration, NaiveDate, NaiveDateTime};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, SubscriptionId};
use common_eventbus::{Event, EventBus, EventBusError, EventData, EventHandler};
use diesel::sql_types::{BigInt, Integer};
use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
use diesel_models::customer_credit::CustomerCreditGrantRow;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::clients::usage::{
    EventSearchOptions, EventSearchResult, GroupedUsageData, IngestEventsRequest,
    IngestEventsResult, MutableMockUsageClient, UsageClient, UsageData, WindowedUsageData,
};
use meteroid_store::domain::checkout_sessions::CheckoutType;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{BillableMetric, CreateCheckoutSession, Invoice, UsagePeriod};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::store::PgPool;
use meteroid_store::{Services, Store};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::data::ids::{
    CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID, METRIC_BANDWIDTH, PLAN_VERSION_PAID_TRIAL_ID,
    PLAN_VERSION_USAGE_ALPHA_ID, PRODUCT_PLATFORM_FEE_ID, TENANT_ID,
};
use crate::harness::{TestEnv, subscription};
use crate::helpers::credit::{Grant, fund_grant};

const READS: u8 = 0;
const FAILS: u8 = 1;
const REDEFINES: u8 = 2;
const NOT_READY: u8 = 3;

/// Metering that answers, fails, or sees the metric redefined after every read (so each pass of
/// locked work misses the read it needs).
struct Metering {
    inner: MutableMockUsageClient,
    mode: AtomicU8,
    pool: OnceLock<PgPool>,
}

impl Metering {
    fn new() -> Arc<Self> {
        let metering = Arc::new(Self {
            inner: MutableMockUsageClient::new(),
            mode: AtomicU8::new(READS),
            pool: OnceLock::new(),
        });
        let at = NaiveDateTime::default();
        metering.inner.set_usage_for_any_nonempty_window(
            METRIC_BANDWIDTH,
            UsageData {
                data: vec![GroupedUsageData {
                    value: Decimal::from(100),
                    dimensions: HashMap::new(),
                }],
                period: UsagePeriod { start: at, end: at },
            },
        );
        metering
    }

    fn set(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl UsageClient for Metering {
    async fn fetch_usage(
        &self,
        tenant_id: &common_domain::ids::TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> meteroid_store::StoreResult<UsageData> {
        match AtomicU8::load(&self.mode, Ordering::SeqCst) {
            FAILS => Err(error_stack::Report::new(StoreError::MeteringServiceError)
                .attach("metering is down")),
            NOT_READY => Err(error_stack::Report::new(StoreError::MeteringNotReady)),
            REDEFINES => {
                let pool = self.pool.get().expect("the pool is set before redefining");
                pool.get()
                    .await
                    .expect("a connection")
                    .batch_execute(&format!(
                        "UPDATE billable_metric SET updated_at = COALESCE(updated_at, now()) + interval '1 second' \
                         WHERE id = '{}'",
                        METRIC_BANDWIDTH.as_uuid()
                    ))
                    .await
                    .expect("the metric is redefined");
                self.inner
                    .fetch_usage(tenant_id, customer_id, metric, period)
                    .await
            }
            _ => {
                self.inner
                    .fetch_usage(tenant_id, customer_id, metric, period)
                    .await
            }
        }
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

/// Usage Alpha (€10 a month, API calls at €0.10) from January 2024, with 100 calls in any window.
async fn alpha(auto_advance: bool) -> (TestEnv, Arc<Metering>, SubscriptionId) {
    let metering = Metering::new();
    let env = crate::harness::test_env_with_usage(metering.clone()).await;
    metering
        .pool
        .set(env.pool().clone())
        .unwrap_or_else(|_| panic!("the pool is set once"));
    env.set_consolidate_and_grace(false, 24).await;
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
    (env, metering, subscription_id)
}

async fn invoice_of(env: &TestEnv, subscription_id: SubscriptionId, month: u32) -> Invoice {
    let start = NaiveDate::from_ymd_opt(2024, month, 1).unwrap();
    env.get_invoices(subscription_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == start)
        .expect("the month's invoice")
}

async fn sql(env: &TestEnv, statement: &str) {
    env.conn()
        .await
        .batch_execute(statement)
        .await
        .unwrap_or_else(|error| panic!("{statement}: {error}"));
}

/// Takes and releases `FOR UPDATE` on the row at once, failing if anyone holds a lock on it.
async fn free(env: &TestEnv, table: &str, id: uuid::Uuid) {
    env.conn()
        .await
        .batch_execute(&format!(
            "BEGIN; SELECT id FROM {table} WHERE id = '{id}' FOR UPDATE NOWAIT; COMMIT;"
        ))
        .await
        .unwrap_or_else(|error| panic!("{table} {id} is held: {error}"));
}

/// Waits until a backend of this database waits on a row lock.
async fn waiting_on_a_lock(env: &TestEnv) {
    let mut conn = env.conn().await;
    for _ in 0..300 {
        let waiting: i64 = diesel::select(diesel::dsl::sql::<BigInt>(
            "(SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock')",
        ))
        .get_result(&mut conn)
        .await
        .expect("read pg_stat_activity");
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("nothing waits on a lock");
}

#[derive(diesel::QueryableByName)]
struct ControllerState {
    #[diesel(sql_type = Integer)]
    attempts: i32,
}

/// The customer's billing controller row: its failed runs.
async fn controller_of(env: &TestEnv) -> ControllerState {
    diesel::sql_query(format!(
        "SELECT attempts FROM billing_controller \
         WHERE tenant_id = '{}' AND customer_id = '{}'",
        TENANT_ID.as_uuid(),
        CUST_UBER_ID.as_uuid()
    ))
    .get_result(&mut env.conn().await)
    .await
    .expect("the customer's billing controller")
}

async fn finalize_after(env: &TestEnv, invoice: &Invoice) -> Option<NaiveDateTime> {
    diesel_models::invoices::InvoiceRow::find_by_id(&mut env.conn().await, TENANT_ID, invoice.id)
        .await
        .expect("the invoice")
        .finalize_after
}

/// Whether a churn movement was booked for the subscription.
async fn churned(env: &TestEnv, subscription_id: SubscriptionId) -> bool {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(subscription_id))
        .select(m::movement_type)
        .load::<diesel_models::enums::MrrMovementType>(&mut env.conn().await)
        .await
        .expect("movements")
        .contains(&diesel_models::enums::MrrMovementType::Churn)
}

/// An insert referencing a row takes KEY SHARE on it: a subscription or grant locked by another
/// transaction (customer first, then the row) must not hold up events, invoices or grant products
/// written against it, else the writer and the lock holder can wait on each other.
#[tokio::test]
async fn rows_referencing_a_locked_subscription_or_grant_never_wait_on_its_lock() {
    let env = crate::harness::test_env_with_usage(Metering::new()).await;
    let subscription_id = subscription().create(env.services()).await;
    let grant = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(1_000),
    )
    .await;

    let mut holder = env.conn().await;
    holder.batch_execute("BEGIN").await.unwrap();
    let lock = SubscriptionRow::lock_subscription_for_update(&mut holder, subscription_id)
        .await
        .expect("the subscription is locked");
    CustomerCreditGrantRow::lock_by_ids(&mut holder, &lock, &[grant])
        .await
        .expect("the grant is locked");

    env.conn()
        .await
        .batch_execute(&format!(
            "SET lock_timeout = '3s'; BEGIN; \
             INSERT INTO subscription_event (id, subscription_id, event_type, created_at, \
               applies_to) \
             VALUES (gen_random_uuid(), '{subscription}', 'SWITCH', now(), '2099-01-01'); \
             INSERT INTO customer_credit_grant_product (tenant_id, grant_id, product_id) \
             VALUES ('{tenant}', '{grant}', '{product}'); \
             ROLLBACK;",
            subscription = subscription_id.as_uuid(),
            tenant = TENANT_ID.as_uuid(),
            grant = grant.as_uuid(),
            product = PRODUCT_PLATFORM_FEE_ID.as_uuid(),
        ))
        .await
        .expect("rows referencing the locked subscription and grant are written without waiting");
    holder.batch_execute("ROLLBACK").await.unwrap();
}

/// A checkout completing takes its customer before its session, as every other path does: while it
/// waits for a customer held elsewhere, it holds no lock on the session.
#[tokio::test]
async fn a_checkout_waits_for_its_customer_before_locking_its_session() {
    let env = crate::harness::test_env_with_usage(Metering::new()).await;
    env.seed_payments().await;
    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_PAID_TRIAL_ID,
            billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
            billing_day_anchor: Some(15),
            net_terms: None,
            trial_duration_days: Some(7),
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("the checkout session");

    let mut holder = env.conn().await;
    holder
        .batch_execute(&format!(
            "BEGIN; SELECT id FROM customer WHERE id = '{}' FOR UPDATE;",
            CUST_UBER_ID.as_uuid()
        ))
        .await
        .expect("another transaction takes the customer");
    let services = env.services().clone();
    let session_id = session.id;
    let completing = tokio::spawn(async move {
        services
            .complete_checkout(
                TENANT_ID,
                session_id,
                CUST_UBER_PAYMENT_METHOD_ID,
                9900,
                "EUR".to_string(),
                None,
            )
            .await
    });
    waiting_on_a_lock(&env).await;
    free(&env, "checkout_session", session.id.as_uuid()).await;
    holder.batch_execute("COMMIT").await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(60), completing)
        .await
        .expect("the checkout completes once the customer is free")
        .expect("the checkout task")
        .expect("the checkout");
}

/// Metering failing between the passes of a step is that subscription's failure (R-121): recorded
/// on the subscription and retried after a backoff, its customer's run completing; the
/// subscription keeps its status and books no churn, however many times it fails.
#[tokio::test]
async fn a_metering_failure_between_step_passes_is_a_controller_failure() {
    let (env, metering, subscription_id) = alpha(true).await;
    let status = env.get_subscription(subscription_id).await.status;
    assert_eq!(status, SubscriptionStatusEnum::Active);
    metering.set(FAILS);

    let before = common_clock::now();
    let run = env
        .services()
        .run_billing_controller(100)
        .await
        .expect("a failing step is recorded, not raised");
    assert_eq!(run.failed, 0, "the customer's run completes");
    assert_eq!(controller_of(&env).await.attempts, 0);
    let failed = env.get_subscription(subscription_id).await;
    assert_eq!(failed.billing_failures, 1);
    assert!(
        failed.next_wake_at.is_some_and(|at| at > before),
        "retried after a backoff"
    );
    assert!(
        failed
            .billing_last_error
            .as_deref()
            .is_some_and(|error| error.contains("metering is down")),
        "{:?}",
        failed.billing_last_error
    );
    assert_eq!(failed.status, status);

    sql(
        &env,
        &format!(
            "UPDATE subscription SET billing_failures = 9, next_wake_at = now() - interval '1 minute' \
             WHERE id = '{}'",
            subscription_id.as_uuid()
        ),
    )
    .await;
    sql(
        &env,
        &format!(
            "UPDATE billing_controller SET next_wake_at = now() - interval '1 minute' \
             WHERE customer_id = '{}'",
            CUST_UBER_ID.as_uuid()
        ),
    )
    .await;
    let before = common_clock::now();
    env.services()
        .run_billing_controller(100)
        .await
        .expect("the tenth failure is recorded, not raised");
    let failed = env.get_subscription(subscription_id).await;
    assert_eq!(failed.billing_failures, 10);
    assert!(
        failed
            .next_wake_at
            .is_some_and(|at| at >= before + Duration::days(1)),
        "retried daily past the fifth"
    );
    assert_eq!(failed.status, status);
    assert!(!churned(&env, subscription_id).await, "no churn booked");
}

/// A metering service that cannot apply usage filters yet (mid-deploy) defers the subscription's
/// step five minutes, counted as no failure.
#[tokio::test]
async fn a_metering_service_without_usage_filters_defers_the_run() {
    let (env, metering, subscription_id) = alpha(true).await;
    metering.set(NOT_READY);

    let before = common_clock::now();
    env.services()
        .run_billing_controller(100)
        .await
        .expect("a deferred step is recorded, not raised");
    let after = common_clock::now();
    let deferred = env.get_subscription(subscription_id).await;
    assert!(
        deferred.next_wake_at.is_some_and(|at| {
            at >= before + Duration::minutes(5) && at <= after + Duration::minutes(5)
        }),
        "tried again in five minutes: {:?}",
        deferred.next_wake_at
    );
    assert_eq!(deferred.billing_failures, 0, "no failure counted");
    assert_eq!(deferred.status, SubscriptionStatusEnum::Active);
}

/// A step whose every pass misses metering (the metric is redefined after each read) runs out of
/// passes: the subscription's failure, recorded and retried.
#[tokio::test]
async fn a_step_out_of_metering_passes_is_a_controller_failure() {
    let (env, metering, subscription_id) = alpha(true).await;
    metering.set(REDEFINES);

    env.services()
        .run_billing_controller(100)
        .await
        .expect("a step out of passes is recorded, not raised");
    let failed = env.get_subscription(subscription_id).await;
    assert_eq!(failed.billing_failures, 1);
    assert!(failed.next_wake_at.is_some());
    assert!(
        failed
            .billing_last_error
            .as_deref()
            .is_some_and(|error| error.contains("kept reading metering")),
        "{:?}",
        failed.billing_last_error
    );
    assert_eq!(failed.status, SubscriptionStatusEnum::Active);
}

/// Metering failing between the passes of a draft's finalization fails the deadline step, logged
/// on its customer while the pass goes on: the draft stays a draft, its `finalize_after` still
/// set, and a later pass finalizes it.
#[tokio::test]
async fn a_metering_failure_between_finalization_passes_leaves_the_deadline_due() {
    let (env, metering, subscription_id) = alpha(true).await;
    env.process_cycle_transitions().await;
    let february = invoice_of(&env, subscription_id, 2).await;
    assert_eq!(february.status, InvoiceStatusEnum::Draft);
    assert!(finalize_after(&env, &february).await.is_some());
    metering.set(FAILS);

    env.services()
        .run_invoice_deadlines()
        .await
        .expect("a failing customer does not stop the pass");
    assert_eq!(
        invoice_of(&env, subscription_id, 2).await.status,
        InvoiceStatusEnum::Draft
    );
    assert!(
        finalize_after(&env, &february).await.is_some(),
        "still due, rolled back with the failed step"
    );

    metering.set(READS);
    env.services()
        .run_invoice_deadlines()
        .await
        .expect("the finalizations run");
    assert_eq!(
        invoice_of(&env, subscription_id, 2).await.status,
        InvoiceStatusEnum::Finalized
    );
    assert_eq!(finalize_after(&env, &february).await, None);
}

#[derive(Default)]
struct RecordingBus(Mutex<Vec<Event>>);

#[async_trait::async_trait]
impl EventBus<Event> for RecordingBus {
    async fn subscribe(&self, _handler: Arc<dyn EventHandler<Event>>) {}

    async fn publish(&self, event: Event) -> Result<(), EventBusError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

impl RecordingBus {
    fn finalized(&self, invoice: &Invoice) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(&event.event_data, EventData::InvoiceFinalized(details)
                    if details.entity_id == invoice.id.as_uuid())
            })
            .count()
    }
}

/// A finalization is announced once, after it commits: its passes that roll back (the first one
/// misses February's usage) announce nothing, and the committed outbox announces it.
#[tokio::test]
async fn an_invoice_is_announced_finalized_only_once_committed() {
    let (env, metering, subscription_id) = alpha(false).await;
    env.process_cycle_transitions().await;
    let january = invoice_of(&env, subscription_id, 1).await;
    env.services()
        .finalize_invoice(Actor::System, january.id, TENANT_ID)
        .await
        .expect("January");
    let february = invoice_of(&env, subscription_id, 2).await;

    let bus = Arc::new(RecordingBus::default());
    let mut store: Store = env.store().clone();
    store.eventbus = bus.clone() as Arc<dyn EventBus<Event>>;
    let services = Services::new(Arc::new(store.clone()), metering.clone());
    let finalized = services
        .finalize_invoice(Actor::System, february.id, TENANT_ID)
        .await
        .expect("February")
        .invoice;
    assert_eq!(finalized.status, InvoiceStatusEnum::Finalized);
    assert_eq!(bus.finalized(&february), 0, "announced before it committed");

    run_once_invoice_orchestration(Arc::new(store), Arc::new(services)).await;
    assert_eq!(bus.finalized(&february), 1);
}
