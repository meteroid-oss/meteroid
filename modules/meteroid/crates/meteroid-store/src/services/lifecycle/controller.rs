//! The billing controller: one writer per customer. A customer runs when its next wake is due or
//! an input changed (`credit_inputs::bump_customer` marks it dirty), under a lease, and takes its
//! due steps in order: each live subscription stepped one boundary a transaction, its transitions,
//! state and documents written there, until it is current; then the invoices' deadlines (finalize
//! after grace, payment attempts), credit expiry, endings still to settle, and drafts to refresh.
//! Its next wake is then the earliest moment any of them is due again; a daily sweep recomputes
//! every wake and counts the lost ones. A failing subscription step is recorded on the
//! subscription and retried on its own, its customer's other work going on; any other failing
//! step is recorded on the customer's row and retried there. Never a status: retried forever.
use crate::StoreResult;
use crate::domain::customer_payment_methods::ResolvedPaymentMethod;
use crate::domain::pgmq::{PaymentRequestEvent, PgmqMessageNew, PgmqQueue};
use crate::errors::StoreError;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::Services;
use crate::services::lifecycle::derived::Issue;
use crate::services::orchestration::payment_transaction_failed::hard_declined_on;
use crate::store::PgConn;
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::billing_controller::BillingControllerRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::InvoiceStatusEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRow;
use diesel_models::query::billing_controller::{DueTimes, Failing};
use diesel_models::query::credit_inputs::{bump_customer, bump_customer_inputs};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use futures::{FutureExt, StreamExt};
use scoped_futures::ScopedFutureExt;

/// How long a run holds its customer: another replica takes it over past it.
const LEASE: Duration = Duration::minutes(5);
/// A run gives up before its lease ends, recorded as a failure.
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);
/// Items a step takes per run, each once.
const STEP_BUDGET: usize = 50;
/// Steps a run takes of one subscription: a late controller writes each missed boundary in turn.
pub(in crate::services) const CATCH_UP: usize = STEP_BUDGET;
/// The soonest a customer runs again when work it could not take is still due (a busy customer,
/// a step failing): never a busy loop.
const RECHECK: Duration = Duration::minutes(1);
const PARALLEL_CUSTOMERS: usize = 4;
/// A draft is refreshed hourly, as the hourly refresh did.
const DRAFT_STALE_HOURS: i32 = 1;
const DRAFT_HORIZON_DAYS: i64 = 7;

static LOST_WAKEUPS: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_controller_lost_wakeups")
            .with_description("Customers the daily sweep found due while their wake was not")
            .build()
    });

static FAILED_RUNS: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_controller_failed_runs")
            .with_description("Customer runs a step failed in, retried on the controller's row")
            .build()
    });

static FAILED_SUBSCRIPTION_STEPS: std::sync::LazyLock<opentelemetry::metrics::Counter<u64>> =
    std::sync::LazyLock::new(|| {
        common_logging::GLOBAL_METER
            .u64_counter("billing_controller_failed_subscription_steps")
            .with_description("Subscription steps that failed, retried on the subscription")
            .build()
    });

/// What alerts: subscriptions and customers failing `ALERT_AFTER` times in a row or more.
static FAILING: std::sync::LazyLock<opentelemetry::metrics::Gauge<u64>> = std::sync::LazyLock::new(
    || {
        common_logging::GLOBAL_METER
            .u64_gauge("billing_controller_failing")
            .with_description(
                "Subscriptions (kind=subscription) and customers (kind=customer) whose billing failed five times in a row or more",
            )
            .build()
    },
);

const ALERT_AFTER: i32 = crate::domain::subscriptions::BILLING_FAILURES_SHOWN;

/// What one pass of the controller did.
#[derive(Debug, Default, Clone, Copy)]
pub struct ControllerRun {
    pub customers: usize,
    pub failed: usize,
}

/// What the daily sweep found.
#[derive(Debug, Default, Clone, Copy)]
pub struct ControllerSweep {
    pub customers: usize,
    pub lost: usize,
    /// Plan windows whose cadence origin is still to decide (a backfill counter, zero once done).
    pub pending_origins: i64,
}

/// Who steps a subscription: a request marks its customer for the controller, the controller's
/// own step does not mark the customer it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::services) enum StepOf {
    Request,
    Controller,
}

/// The steps of a customer's run, each taken on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Steps {
    /// Every step, in order.
    All,
    /// The subscriptions only: their transitions and documents.
    Subscriptions,
    /// The invoices' deadlines only.
    Deadlines,
}

impl Services {
    /// Runs the customers whose wake is due or whose inputs changed, up to `limit`.
    pub(crate) async fn run_billing_controller(&self, limit: i64) -> StoreResult<ControllerRun> {
        self.run_billing_controller_with(limit, CATCH_UP).await
    }

    /// As [`Self::run_billing_controller`], each subscription due stepped up to `catch_up` times.
    pub(crate) async fn run_billing_controller_with(
        &self,
        limit: i64,
        catch_up: usize,
    ) -> StoreResult<ControllerRun> {
        let now = common_clock::now();
        let claimed = {
            let mut conn = self.store.get_conn().await?;
            BillingControllerRow::claim_due(&mut conn, now, now + LEASE, limit.max(1))
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        let customers = claimed.len();
        let failed = futures::stream::iter(claimed)
            .map(|row| self.run_customer_caught(row, catch_up))
            .buffer_unordered(PARALLEL_CUSTOMERS)
            .filter(|ok| futures::future::ready(!ok))
            .count()
            .await;
        Ok(ControllerRun { customers, failed })
    }

    /// [`Self::run_customer`] under a lease renewed as it starts (the customers of a claim run a
    /// few at a time), a panic in it recorded as the customer's failure. True when it did not
    /// fail; a customer another replica took over is left to it.
    async fn run_customer_caught(&self, row: BillingControllerRow, catch_up: usize) -> bool {
        let (tenant_id, customer_id, attempts) = (row.tenant_id, row.customer_id, row.attempts);
        let renewed = async {
            let held = row.lease_until.ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "a claim without a lease".into(),
                ))
            })?;
            let mut conn = self.store.get_conn().await?;
            BillingControllerRow::renew_lease(
                &mut conn,
                tenant_id,
                customer_id,
                held,
                common_clock::now() + LEASE,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)
        }
        .await;
        let lease = match renewed {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                log::warn!(
                    "billing controller of customer {customer_id} taken over before its run"
                );
                return true;
            }
            Err(error) => {
                log::error!(
                    "billing controller lease of customer {customer_id} not renewed: {error:?}"
                );
                return false;
            }
        };
        match std::panic::AssertUnwindSafe(self.run_customer(row, lease, catch_up))
            .catch_unwind()
            .await
        {
            Ok(done) => done,
            Err(panic) => {
                let error = panicked(panic);
                log::error!("billing controller run of customer {customer_id} {error:?}");
                FAILED_RUNS.add(
                    1,
                    &[common_logging::KeyValue::new(
                        "tenant",
                        tenant_id.to_string(),
                    )],
                );
                let now = common_clock::now();
                let recorded = async {
                    let mut conn = self.store.get_conn().await?;
                    BillingControllerRow::fail(
                        &mut conn,
                        tenant_id,
                        customer_id,
                        lease,
                        &format!("{error:?}"),
                        now + retry_after(attempts.saturating_add(1)),
                        now,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                }
                .await;
                if let Err(error) = recorded {
                    log::error!(
                        "billing controller failure of customer {customer_id} not recorded: {error:?}"
                    );
                }
                false
            }
        }
    }

    /// Runs one customer's due steps, then records its next wake. False when it failed.
    async fn run_customer(
        &self,
        row: BillingControllerRow,
        lease: DateTime<Utc>,
        catch_up: usize,
    ) -> bool {
        let (tenant_id, customer_id) = (row.tenant_id, row.customer_id);
        let started = common_clock::now();
        let steps = tokio::time::timeout(
            RUN_TIMEOUT,
            self.customer_steps(tenant_id, customer_id, started, Steps::All, catch_up),
        )
        .await
        .unwrap_or_else(|_| {
            Err(Report::new(StoreError::InvalidArgument(
                "the run outlasted its lease".to_string(),
            )))
        });
        let outcome = match steps {
            Ok(more) => {
                let wake = if more {
                    Ok(Some(started))
                } else {
                    self.next_wake(tenant_id, customer_id)
                        .await
                        .map(|wake| wake.map(|at| at.max(common_clock::now() + RECHECK)))
                };
                match wake {
                    Ok(wake) => {
                        let done = async {
                            let mut conn = self.store.get_conn().await?;
                            BillingControllerRow::complete(
                                &mut conn,
                                tenant_id,
                                customer_id,
                                lease,
                                row.dirty_since,
                                wake,
                                common_clock::now(),
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)
                        }
                        .await;
                        done.map(|held| {
                            if !held {
                                log::warn!(
                                    "billing controller run of customer {customer_id} ended past its lease"
                                );
                            }
                            true
                        })
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        match outcome {
            Ok(done) => done,
            Err(error) => {
                log::error!("billing controller run of customer {customer_id} failed: {error:?}");
                FAILED_RUNS.add(
                    1,
                    &[common_logging::KeyValue::new(
                        "tenant",
                        tenant_id.to_string(),
                    )],
                );
                let now = common_clock::now();
                // The same inputs break the invariant again: the daily sweep looks once more.
                let retry_at = if super::violates_invariant(&error) {
                    now + Duration::days(1)
                } else if super::waits_on_metering(&error) {
                    super::metering_retry_time().and_utc()
                } else {
                    now + retry_after(row.attempts.saturating_add(1))
                };
                let recorded = async {
                    let mut conn = self.store.get_conn().await?;
                    BillingControllerRow::fail(
                        &mut conn,
                        tenant_id,
                        customer_id,
                        lease,
                        &format!("{error:?}"),
                        retry_at,
                        now,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                }
                .await;
                if let Err(error) = recorded {
                    log::error!(
                        "billing controller failure of customer {customer_id} not recorded: {error:?}"
                    );
                }
                if row.attempts.saturating_add(1) >= ALERT_AFTER {
                    self.record_failing().await;
                }
                false
            }
        }
    }

    /// The customer's due steps, in order, each subscription stepped up to `catch_up` times; true
    /// when a step took work, which may have made more due (the customer then runs again at
    /// once). A failing item does not stop the others: the first failure is returned once every
    /// step ran.
    pub(crate) async fn customer_steps(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        now: DateTime<Utc>,
        steps: Steps,
        catch_up: usize,
    ) -> StoreResult<bool> {
        let customer = (tenant_id, customer_id);
        let mut failed: Option<Report<StoreError>> = None;
        let mut more = false;
        if steps != Steps::Deadlines {
            more |= self
                .subscription_steps_of(customer, now, catch_up, &mut failed)
                .await?;
        }
        if steps != Steps::Subscriptions {
            more |= self
                .invoice_deadlines_of(customer, now, &mut failed)
                .await?;
        }
        // Credit, endings and drafts see the customer once every due transition is applied: a
        // subscription ending is ended before its credit lapses as an expiry.
        if steps == Steps::All && !more {
            self.credit_due_of(customer, now).await?;
            self.settle_missed_endings_of(Some(customer_id), STEP_BUDGET as i64)
                .await?;
            more |= self.refresh_drafts_of(customer).await?;
        }
        match failed {
            Some(error) => Err(error),
            None => Ok(more),
        }
    }

    /// Each live subscription due stepped until a step takes no work, up to `catch_up` steps,
    /// each in its own transaction: its transitions, state and documents at the clock, one
    /// boundary a step. True when one took work.
    async fn subscription_steps_of(
        &self,
        (tenant_id, customer_id): (TenantId, CustomerId),
        now: DateTime<Utc>,
        catch_up: usize,
        failed: &mut Option<Report<StoreError>>,
    ) -> StoreResult<bool> {
        let due = {
            let mut conn = self.store.get_conn().await?;
            BillingControllerRow::due_subscriptions_of(
                &mut conn,
                tenant_id,
                customer_id,
                now,
                STEP_BUDGET as i64,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        };
        let mut took = false;
        for subscription_id in due {
            let stepped = std::panic::AssertUnwindSafe(self.step_until_idle(
                tenant_id,
                subscription_id,
                catch_up,
                StepOf::Controller,
            ))
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| Err(panicked(panic)));
            let recorded = match stepped {
                Ok(work) => {
                    took |= work;
                    self.subscription_took(subscription_id).await
                }
                Err(error) => {
                    log::error!("subscription {subscription_id} not stepped: {error:?}");
                    self.subscription_failed(tenant_id, subscription_id, &error)
                        .await
                }
            };
            if let Err(error) = recorded {
                failed.get_or_insert(error);
            }
        }
        Ok(took)
    }

    /// A subscription's step failed: it alone waits for its retry, the invariant's after a day
    /// (R-095), the metering's without counting.
    async fn subscription_failed(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        error: &Report<StoreError>,
    ) -> StoreResult<()> {
        let now = common_clock::now();
        let mut conn = self.store.get_conn().await?;
        if super::waits_on_metering(error) {
            let retry_at = super::metering_retry_time().and_utc();
            return SubscriptionRow::set_next_wake_at(&mut conn, subscription_id, Some(retry_at))
                .await
                .map_err(Into::<Report<StoreError>>::into);
        }
        let failures = SubscriptionRow::billing_failures(&mut conn, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .saturating_add(1);
        let retry_at = if super::violates_invariant(error) {
            now + Duration::days(1)
        } else {
            now + retry_after(failures)
        };
        SubscriptionRow::record_billing_failure(
            &mut conn,
            subscription_id,
            &format!("{error:?}"),
            retry_at,
            now,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        FAILED_SUBSCRIPTION_STEPS.add(
            1,
            &[common_logging::KeyValue::new(
                "tenant",
                tenant_id.to_string(),
            )],
        );
        if failures >= ALERT_AFTER {
            log::error!(
                "billing of subscription {subscription_id} failed {failures} times in a row, retried at {retry_at}"
            );
            drop(conn);
            self.record_failing().await;
        }
        Ok(())
    }

    /// A subscription's step took: what it failed before is cleared.
    async fn subscription_took(&self, subscription_id: SubscriptionId) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        let cleared = SubscriptionRow::clear_billing_failures(&mut conn, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        drop(conn);
        if cleared {
            self.record_failing().await;
        }
        Ok(())
    }

    /// Exports what is failing past `ALERT_AFTER`, for the alert.
    pub(crate) async fn record_failing(&self) {
        let counted = async {
            let mut conn = self.store.get_conn().await?;
            Failing::count(&mut conn, ALERT_AFTER)
                .await
                .map_err(Into::<Report<StoreError>>::into)
        }
        .await;
        match counted {
            Ok(failing) => {
                FAILING.record(
                    u64::try_from(failing.subscriptions).unwrap_or(0),
                    &[common_logging::KeyValue::new("kind", "subscription")],
                );
                FAILING.record(
                    u64::try_from(failing.customers).unwrap_or(0),
                    &[common_logging::KeyValue::new("kind", "customer")],
                );
            }
            Err(error) => log::warn!("billing failures not counted: {error:?}"),
        }
    }

    /// The subscription stepped until a step takes no work or it is no longer live, up to
    /// `steps` steps; true when one took work.
    pub(in crate::services) async fn step_until_idle(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        steps: usize,
        of: StepOf,
    ) -> StoreResult<bool> {
        let mut took = false;
        for _ in 0..steps {
            if !self
                .subscription_step(tenant_id, subscription_id, of)
                .await?
            {
                break;
            }
            took = true;
        }
        Ok(took)
    }

    /// One subscription's step at the clock, under its customer's lock, reading metering first.
    /// True when it took work.
    pub(in crate::services) async fn subscription_step(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        of: StepOf,
    ) -> StoreResult<bool> {
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                let lock = SubscriptionRow::lock_customer_of(conn, subscription_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let stepped = services
                    .step_subscription_tx(
                        conn,
                        &common_domain::actor::Actor::System,
                        tenant_id,
                        subscription_id,
                        common_clock::now(),
                        Issue::AfterGrace,
                    )
                    .await?;
                let bumped = match of {
                    StepOf::Request => bump_customer(conn, &lock).await,
                    StepOf::Controller => bump_customer_inputs(conn, &lock).await,
                };
                bumped.map_err(Into::<Report<StoreError>>::into)?;
                Ok(stepped.is_some_and(|stepped| stepped.took_work()))
            }
            .scope_boxed()
        })
        .await
    }

    /// The customer's drafts whose grace ended, finalized (merged with their same-day siblings
    /// when the entity consolidates), then the payment attempts due. True when it took any.
    async fn invoice_deadlines_of(
        &self,
        (tenant_id, customer_id): (TenantId, CustomerId),
        now: DateTime<Utc>,
        failed: &mut Option<Report<StoreError>>,
    ) -> StoreResult<bool> {
        let (finalizations, attempts) = {
            let mut conn = self.store.get_conn().await?;
            let finalizations = InvoiceRow::list_due_finalizations_of(
                &mut conn,
                tenant_id,
                customer_id,
                now.naive_utc(),
                STEP_BUDGET as i64,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            let attempts = InvoiceRow::list_due_payment_attempts_of(
                &mut conn,
                tenant_id,
                customer_id,
                now.naive_utc(),
                STEP_BUDGET as i64,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            (finalizations, attempts)
        };
        // Only what was taken makes more due: a deadline failing at every run never holds back
        // the customer's credit, endings and drafts.
        let mut took = false;
        for invoice_id in finalizations {
            match self.finalize_after_grace(tenant_id, invoice_id).await {
                Ok(()) => took = true,
                Err(error) => {
                    log::error!("draft {invoice_id} not finalized: {error:?}");
                    failed.get_or_insert(error);
                }
            }
        }
        for (invoice_id, subscription_id) in attempts {
            let attempted = self
                .store
                .transaction(|conn| {
                    async move {
                        let lock = InvoiceRow::lock_customer_of(conn, tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        // Another run may have taken it since it was listed.
                        if InvoiceRow::take_due_payment_attempt(
                            conn,
                            tenant_id,
                            invoice_id,
                            now.naive_utc(),
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        {
                            self.attempt_payment_tx(conn, tenant_id, invoice_id, subscription_id)
                                .await?;
                        }
                        bump_customer_inputs(conn, &lock)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)
                    }
                    .scope_boxed()
                })
                .await;
            match attempted {
                Ok(()) => took = true,
                Err(error) => {
                    log::error!("payment of invoice {invoice_id} not attempted: {error:?}");
                    failed.get_or_insert(error);
                }
            }
        }
        Ok(took)
    }

    /// A draft whose grace ended: finalized, or merged into its customer's consolidated invoice.
    async fn finalize_after_grace(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                let lock = InvoiceRow::lock_customer_of(conn, tenant_id, invoice_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                // Another run may have taken it since it was listed.
                if !InvoiceRow::take_due_finalization(
                    conn,
                    tenant_id,
                    invoice_id,
                    common_clock::now().naive_utc(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                {
                    return Ok(());
                }
                services
                    .consolidate_and_finalize(conn, tenant_id, invoice_id)
                    .await?;
                bump_customer_inputs(conn, &lock)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
            }
            .scope_boxed()
        })
        .await
    }

    /// Dunning: collects again an invoice whose previous payment failed, through the same
    /// durable payment path as an automatic first charge. The next attempt is set by the failure
    /// handler when this one fails in turn.
    async fn attempt_payment_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        subscription_id: Option<SubscriptionId>,
    ) -> StoreResult<()> {
        let Some(subscription_id) = subscription_id else {
            return Ok(());
        };
        let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        // Written off or cancelled since.
        if invoice.status != InvoiceStatusEnum::Finalized {
            return Ok(());
        }
        // Collected since, or someone got there first (portal payment, manual charge).
        if !matches!(
            crate::repositories::invoice_account::charge_tx(conn, &invoice).await?,
            billing_core::settlement::Charge::Due(_)
        ) {
            return Ok(());
        }
        let ResolvedPaymentMethod::CustomerPaymentMethod(payment_method_id) = self
            .store
            .resolve_payment_method_for_subscription(tenant_id, subscription_id)
            .await?
        else {
            log::info!("No automatic payment method for invoice {invoice_id}; stopping dunning");
            return Ok(());
        };
        // A hard decline since: only a new method (or refreshed card details) revives it.
        let latest_failure =
            PaymentTransactionRow::latest_failed_for_invoice(conn, invoice_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
        let method = CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &payment_method_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if latest_failure.is_some_and(|failed| hard_declined_on(&failed, &method)) {
            log::info!("Payment method of invoice {invoice_id} was hard-declined; skipping retry");
            return Ok(());
        }
        let msg: PgmqMessageNew =
            PaymentRequestEvent::new(tenant_id, invoice_id, payment_method_id).try_into()?;
        // A retried collection is still an auto-charge decision for this invoice.
        if invoice.auto_charge_requested_at.is_none() {
            InvoiceRow::set_auto_charge_requested(
                conn,
                invoice_id,
                tenant_id,
                Some(common_clock::now().naive_utc()),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        }
        self.store
            .pgmq_send_batch_tx(conn, PgmqQueue::PaymentRequest, vec![msg])
            .await?;
        Ok(())
    }

    async fn credit_due_of(
        &self,
        (tenant_id, customer_id): (TenantId, CustomerId),
        now: DateTime<Utc>,
    ) -> StoreResult<()> {
        let notes = {
            let mut conn = self.store.get_conn().await?;
            diesel_models::customer_credit::CustomerCreditGrantRow::list_forgiven_purchase_notes_of(
                &mut conn,
                tenant_id,
                customer_id,
                now,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        };
        for note in notes {
            if let Err(error) = self.credit_back_forgiven_late_packs(tenant_id, note).await {
                log::error!("late pack of forgiven note {note} not credited back: {error:?}");
            }
        }
        // As the sweep did: credit of a subscription that ended is ended with it, then the rest
        // that expired lapses.
        crate::services::credit::subscription_end::end_credit_of_ended_subscriptions_of(
            &self.store,
            STEP_BUDGET as i64,
            Some(customer_id),
        )
        .await?;
        crate::services::credit::expiry_settlement::lapse_expired_credit_of(
            &self.store,
            tenant_id,
            customer_id,
            now,
        )
        .await?;
        Ok(())
    }

    async fn refresh_drafts_of(
        &self,
        (tenant_id, customer_id): (TenantId, CustomerId),
    ) -> StoreResult<bool> {
        let drafts = {
            let mut conn = self.store.get_conn().await?;
            diesel_models::invoices::InvoiceRow::list_refreshable_drafts_of(
                &mut conn,
                tenant_id,
                customer_id,
                DRAFT_STALE_HOURS,
                DRAFT_HORIZON_DAYS,
                STEP_BUDGET as i64,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        };
        let full = drafts.len() >= STEP_BUDGET;
        let mut refreshed = 0;
        for invoice_id in drafts {
            let mut conn = self.store.get_conn().await?;
            match self
                .refresh_draft_reading_usage_first(&mut conn, invoice_id, tenant_id)
                .await
            {
                Ok(_) => refreshed += 1,
                Err(error) => log::warn!("draft {invoice_id} not refreshed: {error:?}"),
            }
        }
        // Failing refreshes (metering down) wait for the recheck, never a run at once.
        Ok(full && refreshed > 0)
    }

    /// The earliest moment one of the customer's steps is due, None when none ever is.
    pub(crate) async fn next_wake(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Option<DateTime<Utc>>> {
        let now = common_clock::now();
        let due = {
            let mut conn = self.store.get_conn().await?;
            DueTimes::of_customer(&mut conn, tenant_id, customer_id, now)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        Ok(wake_of(&due, now))
    }

    /// One pass of `steps` over every customer a run may take, as a run would, leases aside,
    /// each subscription stepped once: tests and tools driving the steps on their own. A failing
    /// customer is logged and the others run. Returns how many took work.
    pub(crate) async fn run_steps_of_all(&self, steps: Steps) -> StoreResult<usize> {
        let mut after = None;
        let mut took = 0;
        loop {
            let customers = {
                let mut conn = self.store.get_conn().await?;
                BillingControllerRow::list_runnable_customers(&mut conn, after, 500)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
            };
            let Some(last) = customers.last().copied() else {
                return Ok(took);
            };
            after = Some(last);
            for (tenant_id, customer_id) in customers {
                match self
                    .customer_steps(tenant_id, customer_id, common_clock::now(), steps, 1)
                    .await
                {
                    Ok(true) => took += 1,
                    Ok(false) => {}
                    Err(error) => {
                        log::error!("steps of customer {customer_id} failed: {error:?}");
                    }
                }
            }
        }
    }

    /// Recomputes every customer's wake; counts those due whose stored wake was not.
    pub(crate) async fn sweep_billing_controller(&self) -> StoreResult<ControllerSweep> {
        let mut sweep = ControllerSweep::default();
        let mut after = None;
        loop {
            let customers = {
                let mut conn = self.store.get_conn().await?;
                BillingControllerRow::list_customers(&mut conn, after, 500)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
            };
            let Some(last) = customers.last().copied() else {
                self.record_failing().await;
                // After the wakes: an origin backfill failing never holds them back.
                match self.decide_pending_window_origins().await {
                    Ok(pending) => sweep.pending_origins = pending,
                    Err(error) => log::error!("cadence origins not decided: {error:?}"),
                }
                if sweep.pending_origins > 0 {
                    log::warn!(
                        "{} plan windows still have no cadence origin",
                        sweep.pending_origins
                    );
                }
                return Ok(sweep);
            };
            after = Some(last);
            let mut lost = 0;
            for (tenant_id, customer_id) in customers {
                sweep.customers += 1;
                let now = common_clock::now();
                let rewoken = async {
                    let wake = self.next_wake(tenant_id, customer_id).await?;
                    let mut conn = self.store.get_conn().await?;
                    BillingControllerRow::rewake(&mut conn, tenant_id, customer_id, wake, now)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .map(|previous| (wake, previous))
                }
                .await;
                // One customer failing is logged; the sweep goes on to the others.
                let (wake, previous) = match rewoken {
                    Ok(rewoken) => rewoken,
                    Err(error) => {
                        log::error!(
                            "billing controller wake of customer {customer_id} not recomputed: {error:?}"
                        );
                        continue;
                    }
                };
                let due = wake.is_some_and(|at| at <= now);
                // A dirty customer runs anyway: its wake is not lost.
                if let Some((previous, false)) = previous
                    && due
                    && previous.is_none_or(|at| at > now)
                {
                    lost += 1;
                    log::warn!("billing controller of customer {customer_id} lost its wake");
                }
            }
            sweep.lost += lost;
            if lost > 0 {
                LOST_WAKEUPS.add(lost as u64, &[]);
            }
        }
    }
}

/// The earliest of the due times, a credit lapse waiting its settlement grace.
fn wake_of(due: &DueTimes, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let lapse = due.lapse.map(|lapse| {
        let grace = crate::repositories::credit::customer_credit::settlement_grace(
            due.grace_period_hours.unwrap_or(0),
            due.consolidate_recurring_invoices.unwrap_or(false),
        );
        // A forgiven late pack is credited back at the lapse, the rest settled after the grace.
        if lapse > now { lapse } else { lapse + grace }
    });
    [
        due.derive,
        due.finalize,
        due.attempt,
        lapse,
        due.ending,
        due.draft,
    ]
    .into_iter()
    .flatten()
    .min()
}

/// A panic of a run or a step, as its failure.
fn panicked(panic: Box<dyn std::any::Any + Send>) -> Report<StoreError> {
    let message = panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".to_owned());
    Report::new(StoreError::BillingError).attach(format!("panicked: {message}"))
}

/// A failing run or step waits longer each time: 4, 8, 16, 32 and 60 minutes, then a day, forever.
fn retry_after(failures: i32) -> Duration {
    if failures > ALERT_AFTER {
        return Duration::days(1);
    }
    let minutes = 1_i64 << (failures.clamp(1, ALERT_AFTER) + 1);
    Duration::minutes(minutes.min(60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_lapse_wakes_at_once_then_after_its_settlement_grace() {
        let now = Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
        let mut due = DueTimes {
            lapse: Some(now + Duration::hours(2)),
            grace_period_hours: Some(1),
            ..Default::default()
        };
        assert_eq!(wake_of(&due, now), Some(now + Duration::hours(2)));
        due.lapse = Some(now - Duration::minutes(10));
        assert_eq!(wake_of(&due, now), Some(now + Duration::minutes(50)));
        due.derive = Some(now + Duration::minutes(5));
        assert_eq!(wake_of(&due, now), Some(now + Duration::minutes(5)));
    }

    /// A draft's grace and a payment attempt wake the customer as a subscription's wake does.
    #[test]
    fn an_invoice_deadline_wakes_the_customer() {
        let now = Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
        let mut due = DueTimes {
            derive: Some(now + Duration::days(3)),
            finalize: Some(now + Duration::hours(6)),
            ..Default::default()
        };
        assert_eq!(wake_of(&due, now), Some(now + Duration::hours(6)));
        due.attempt = Some(now + Duration::hours(1));
        assert_eq!(wake_of(&due, now), Some(now + Duration::hours(1)));
    }

    /// Rules: R-121
    #[test]
    fn a_failing_run_backs_off_up_to_an_hour_then_retries_daily() {
        let waits: Vec<i64> = (1..=7).map(|n| retry_after(n).num_minutes()).collect();
        assert_eq!(waits, vec![4, 8, 16, 32, 60, 1440, 1440]);
        assert_eq!(retry_after(1000), Duration::days(1));
    }
}
