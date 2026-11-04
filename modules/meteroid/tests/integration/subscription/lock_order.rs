//! Row locks never form a cycle with the customer lock, a busy customer never stalls other
//! renewals, and metering failing between the passes of locked work is recorded like any failure.
use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, SubscriptionId};
use common_eventbus::{Event, EventBus, EventBusError, EventData, EventHandler};
use diesel::sql_types::{BigInt, Integer, Nullable, Text, Timestamp};
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
use meteroid_store::domain::enums::{CustomerType, InvoiceStatusEnum};
use meteroid_store::domain::{
    BillableMetric, CreateCheckoutSession, CustomerNew, CustomerTaxStatus, Invoice, UsagePeriod,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::store::PgPool;
use meteroid_store::{Services, Store};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::data::ids::{
    CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID, METRIC_BANDWIDTH, PLAN_VERSION_PAID_TRIAL_ID,
    PLAN_VERSION_USAGE_ALPHA_ID, PRODUCT_PLATFORM_FEE_ID, TENANT_ID, USER_ID,
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
struct EventState {
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Integer)]
    retries: i32,
    #[diesel(sql_type = Nullable<Text>)]
    error: Option<String>,
    #[diesel(sql_type = Timestamp)]
    scheduled_time: NaiveDateTime,
    #[diesel(sql_type = Nullable<Timestamp>)]
    next_attempt_at: Option<NaiveDateTime>,
}

async fn finalization_event(env: &TestEnv, invoice: &Invoice) -> EventState {
    diesel::sql_query(format!(
        "SELECT status::text AS status, retries, error, scheduled_time, next_attempt_at \
         FROM scheduled_event \
         WHERE event_type = 'FINALIZE_INVOICE' AND event_data::text LIKE '%{}%'",
        invoice.id.as_uuid()
    ))
    .get_result(&mut env.conn().await)
    .await
    .expect("the draft's scheduled finalization")
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
             INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, \
               scheduled_time, priority, event_data, created_at, updated_at, status, retries, \
               source, created_by_customer, actor_type) \
             VALUES (gen_random_uuid(), '{subscription}', '{tenant}', 'CANCEL_SUBSCRIPTION', \
               '2099-01-01', 0, '{{}}', now(), now(), 'CANCELED', 0, 'test', false, 'SYSTEM'); \
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

/// Metering failing between the passes of a renewal is a failed renewal: counted, retried with a
/// backoff, and Errored after the last retry, as when it failed under the locks.
#[tokio::test]
async fn a_metering_failure_between_renewal_passes_is_a_cycle_error() {
    let (env, metering, subscription_id) = alpha(true).await;
    metering.set(FAILS);

    env.services()
        .get_and_process_cycle_transitions()
        .await
        .expect("a failing renewal is recorded, not raised");
    let failed = env.get_subscription(subscription_id).await;
    assert_eq!(failed.error_count, 1);
    assert!(
        failed
            .next_retry
            .is_some_and(|at| at > Utc::now().naive_utc()),
        "retried after a backoff"
    );
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("metering is down")),
        "{:?}",
        failed.last_error
    );
    assert_ne!(failed.status, SubscriptionStatusEnum::Errored);

    sql(
        &env,
        &format!(
            "UPDATE subscription SET error_count = 9, next_retry = now() - interval '1 minute', \
             processing_started_at = NULL WHERE id = '{}'",
            subscription_id.as_uuid()
        ),
    )
    .await;
    env.services()
        .get_and_process_cycle_transitions()
        .await
        .expect("the last retry is recorded, not raised");
    let errored = env.get_subscription(subscription_id).await;
    assert_eq!(errored.error_count, 10);
    assert_eq!(errored.status, SubscriptionStatusEnum::Errored);
}

/// A metering service that cannot apply usage filters yet (mid-deploy) defers the renewal without
/// counting a failure: the subscription never runs out of retries over it.
#[tokio::test]
async fn a_metering_service_without_usage_filters_defers_the_renewal() {
    let (env, metering, subscription_id) = alpha(true).await;
    metering.set(NOT_READY);
    sql(
        &env,
        &format!(
            "UPDATE subscription SET error_count = 9 WHERE id = '{}'",
            subscription_id.as_uuid()
        ),
    )
    .await;

    env.services()
        .get_and_process_cycle_transitions()
        .await
        .expect("a deferred renewal is recorded, not raised");
    let deferred = env.get_subscription(subscription_id).await;
    assert_eq!(deferred.error_count, 9, "not counted as a failure");
    assert_ne!(deferred.status, SubscriptionStatusEnum::Errored);
    assert!(
        deferred
            .next_retry
            .is_some_and(|at| at > Utc::now().naive_utc()),
        "tried again later"
    );
    assert!(
        deferred
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("usage filters")),
        "{:?}",
        deferred.last_error
    );
}

/// A renewal whose every pass misses metering (the metric is redefined after each read) runs out
/// of passes: a failed renewal, recorded and retried.
#[tokio::test]
async fn a_renewal_out_of_metering_passes_is_a_cycle_error() {
    let (env, metering, subscription_id) = alpha(true).await;
    metering.set(REDEFINES);

    env.services()
        .get_and_process_cycle_transitions()
        .await
        .expect("a renewal out of passes is recorded, not raised");
    let failed = env.get_subscription(subscription_id).await;
    assert_eq!(failed.error_count, 1);
    assert!(failed.next_retry.is_some());
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("kept reading metering")),
        "{:?}",
        failed.last_error
    );
}

/// Metering failing between the passes of a scheduled finalization fails the event: retried,
/// then Failed after the last retry, never left Processing.
#[tokio::test]
async fn a_metering_failure_between_scheduled_finalization_passes_fails_the_event() {
    let (env, metering, subscription_id) = alpha(true).await;
    env.process_cycle_transitions().await;
    let february = invoice_of(&env, subscription_id, 2).await;
    assert_eq!(february.status, InvoiceStatusEnum::Draft);
    metering.set(FAILS);

    env.services()
        .get_and_process_due_events()
        .await
        .expect("the events run");
    let retried = finalization_event(&env, &february).await;
    assert_eq!(
        (retried.status.as_str(), retried.retries),
        ("PENDING", 1),
        "{:?}",
        retried.error
    );
    assert!(
        retried
            .error
            .as_deref()
            .is_some_and(|error| error.contains("metering is down"))
    );

    sql(
        &env,
        &format!(
            "UPDATE scheduled_event SET retries = 5, next_attempt_at = now() - interval '1 minute' \
             WHERE event_type = 'FINALIZE_INVOICE' AND event_data::text LIKE '%{}%'",
            february.id.as_uuid()
        ),
    )
    .await;
    env.services()
        .get_and_process_due_events()
        .await
        .expect("the events run");
    assert_eq!(finalization_event(&env, &february).await.status, "FAILED");
    assert_eq!(
        invoice_of(&env, subscription_id, 2).await.status,
        InvoiceStatusEnum::Draft
    );
}

/// A retried event keeps the date it takes effect on (a cancellation or pause reads it as its
/// effective date): only its next attempt moves, and it is not claimed before then.
#[tokio::test]
async fn a_retried_event_keeps_its_effective_date() {
    let (env, metering, subscription_id) = alpha(true).await;
    env.process_cycle_transitions().await;
    let february = invoice_of(&env, subscription_id, 2).await;
    let due = finalization_event(&env, &february).await.scheduled_time;
    metering.set(FAILS);

    env.services()
        .get_and_process_due_events()
        .await
        .expect("the events run");
    let retried = finalization_event(&env, &february).await;
    assert_eq!((retried.status.as_str(), retried.retries), ("PENDING", 1));
    assert_eq!(retried.scheduled_time, due, "the effective date stays");
    assert!(
        retried
            .next_attempt_at
            .is_some_and(|at| at > Utc::now().naive_utc())
    );

    metering.set(READS);
    env.services()
        .get_and_process_due_events()
        .await
        .expect("the events run");
    assert_eq!(
        finalization_event(&env, &february).await.retries,
        1,
        "not claimed before its next attempt"
    );
    sql(
        &env,
        &format!(
            "UPDATE scheduled_event SET next_attempt_at = now() - interval '1 minute' \
             WHERE event_type = 'FINALIZE_INVOICE' AND event_data::text LIKE '%{}%'",
            february.id.as_uuid()
        ),
    )
    .await;
    env.services()
        .get_and_process_due_events()
        .await
        .expect("the events run");
    let done = finalization_event(&env, &february).await;
    assert_eq!(done.status, "COMPLETED");
    assert_eq!(done.scheduled_time, due);
    assert_eq!(
        invoice_of(&env, subscription_id, 2).await.status,
        InvoiceStatusEnum::Finalized
    );
}

/// A metering service that cannot apply usage filters yet defers a scheduled event without
/// counting a failure, as it does a renewal: an event on its last retry is not failed over it.
#[tokio::test]
async fn a_metering_service_without_usage_filters_defers_the_event_uncounted() {
    let (env, metering, subscription_id) = alpha(true).await;
    env.process_cycle_transitions().await;
    let february = invoice_of(&env, subscription_id, 2).await;
    let due = finalization_event(&env, &february).await.scheduled_time;
    sql(
        &env,
        &format!(
            "UPDATE scheduled_event SET retries = 5 \
             WHERE event_type = 'FINALIZE_INVOICE' AND event_data::text LIKE '%{}%'",
            february.id.as_uuid()
        ),
    )
    .await;
    metering.set(NOT_READY);

    env.services()
        .get_and_process_due_events()
        .await
        .expect("the events run");
    let deferred = finalization_event(&env, &february).await;
    assert_eq!((deferred.status.as_str(), deferred.retries), ("PENDING", 5));
    assert_eq!(deferred.scheduled_time, due);
    assert!(
        deferred
            .next_attempt_at
            .is_some_and(|at| at > Utc::now().naive_utc()),
        "tried again later"
    );
    assert!(
        deferred
            .error
            .as_deref()
            .is_some_and(|error| error.contains("usage filters")),
        "{:?}",
        deferred.error
    );
}

/// A cancellation dated Jan 31 in retry backoff across the Feb 1 renewal: the renewal waits for it,
/// so February is never billed and the cancellation takes effect on Jan 31.
#[tokio::test]
async fn a_renewal_waits_for_a_change_dated_in_its_period() {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;

    let (env, _metering, subscription_id) = alpha(true).await;
    let cancel_on = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
    env.services()
        .cancel_subscription(
            Actor::System,
            subscription_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(cancel_on),
        )
        .await
        .expect("cancellation scheduled");
    let cancellation = |set: &str| {
        format!(
            "UPDATE scheduled_event SET {set} \
             WHERE event_type = 'CANCEL_SUBSCRIPTION' AND subscription_id = '{}'",
            subscription_id.as_uuid()
        )
    };
    sql(
        &env,
        &cancellation("retries = 1, next_attempt_at = now() + interval '1 hour'"),
    )
    .await;

    env.services()
        .get_and_process_cycle_transitions()
        .await
        .expect("a waiting renewal is recorded, not raised");
    let waiting = env.get_subscription(subscription_id).await;
    assert_eq!(
        waiting.current_period_start,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        "not renewed"
    );
    assert_eq!(waiting.error_count, 0, "not a failure");
    assert!(
        waiting
            .next_retry
            .is_some_and(|at| at > Utc::now().naive_utc() + Duration::minutes(59))
    );
    assert!(
        waiting
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("has not applied yet")),
        "{:?}",
        waiting.last_error
    );

    sql(
        &env,
        &cancellation("next_attempt_at = now() - interval '1 minute'"),
    )
    .await;
    env.services()
        .get_and_process_due_events()
        .await
        .expect("the events run");
    env.services()
        .get_and_process_cycle_transitions()
        .await
        .expect("the cycle runs");
    let cancelled = env.get_subscription(subscription_id).await;
    assert_eq!(cancelled.status, SubscriptionStatusEnum::Cancelled);
    assert_eq!(
        cancelled.current_period_start, cancel_on,
        "effective on Jan 31"
    );
    assert!(
        env.get_invoices(subscription_id)
            .await
            .iter()
            .all(|invoice| invoice.billing_period_start
                < NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()),
        "February is never billed"
    );
}

/// Eight customers held by other transactions own the oldest-due renewals: the worker defers them
/// (no error, a short jittered retry) instead of claiming them first again, so a ninth customer's
/// renewal goes through on the same tick or the next.
#[tokio::test]
async fn busy_customers_never_stall_the_renewals_behind_them() {
    let env = crate::harness::test_env_with_usage(Metering::new()).await;
    let mut busy = Vec::new();
    for index in 0..8 {
        let customer = env
            .store()
            .insert_customer(
                meteroid_store::domain::entity_activity::Actor::User { id: USER_ID },
                CustomerNew {
                    name: format!("Busy {index}"),
                    customer_type: CustomerType::Company,
                    first_name: None,
                    last_name: None,
                    alias: None,
                    billing_email: None,
                    invoicing_emails: vec![format!("busy{index}@example.test")],
                    phone: None,
                    currency: "EUR".to_string(),
                    billing_address: None,
                    shipping_address: None,
                    invoicing_entity_id: None,
                    force_created_date: None,
                    tax_status: CustomerTaxStatus::Taxable,
                    exemption_reason: None,
                    vat_number: None,
                    legal_number: None,
                    buyer_reference: None,
                    custom_taxes: vec![],
                    connected_account_id: None,
                    custom_properties: serde_json::json!({}),
                    preferred_locales: vec![],
                },
                TENANT_ID,
            )
            .await
            .expect("a customer")
            .id;
        let subscription_id = subscription()
            .customer(customer)
            .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
            .no_auto_advance()
            .create(env.services())
            .await;
        busy.push((customer, subscription_id));
    }
    let free_subscription = subscription()
        .start_date(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
        .no_auto_advance()
        .create(env.services())
        .await;
    let free_period_end = env
        .get_subscription(free_subscription)
        .await
        .current_period_end;

    let mut holder = env.conn().await;
    let customers = busy
        .iter()
        .map(|(customer, _)| format!("'{}'", customer.as_uuid()))
        .collect::<Vec<_>>()
        .join(", ");
    holder
        .batch_execute(&format!(
            "BEGIN; SELECT id FROM customer WHERE id IN ({customers}) FOR UPDATE;"
        ))
        .await
        .expect("other transactions take the eight customers");

    let before = Utc::now().naive_utc();
    let mut ticks = 0;
    while env
        .get_subscription(free_subscription)
        .await
        .current_period_end
        == free_period_end
    {
        assert!(
            ticks < 2,
            "the free customer's renewal waits behind the busy customers"
        );
        ticks += 1;
        // A tick of the worker: it claims again at once while a batch comes back full.
        for _ in 0..10 {
            let tick = env
                .services()
                .get_and_process_cycle_transitions()
                .await
                .expect("a tick");
            if !tick.has_more {
                break;
            }
        }
    }

    for (_, subscription_id) in &busy {
        let deferred = env.get_subscription(*subscription_id).await;
        assert_eq!(deferred.error_count, 0, "a busy customer is not a failure");
        assert!(deferred.processing_started_at.is_none());
        let retry = deferred.next_retry.expect("deferred");
        assert!(
            retry >= before + Duration::seconds(30)
                && retry <= Utc::now().naive_utc() + Duration::seconds(90),
            "deferred by 30 to 90 seconds: {retry}"
        );
    }
    holder.batch_execute("COMMIT").await.unwrap();
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
