//! The billing controller (Phase 6): one writer per customer. A customer runs when its next wake
//! is due or an input changed (`credit_inputs::bump_customer` marks it dirty), under a lease, and
//! takes its due steps in order, each by the function its own request path uses: scheduled
//! changes, then renewals, then what they made due (finalize after grace, payment retries), credit
//! expiry, endings still to settle, and drafts to refresh. Its next wake is then the earliest
//! moment any of them is due again; a daily sweep recomputes every wake and counts the lost ones.
use crate::StoreResult;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::lifecycle::period_transitions::Visit;
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{CustomerId, TenantId};
use diesel_models::billing_controller::BillingControllerRow;
use diesel_models::query::billing_controller::DueTimes;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use futures::StreamExt;

/// How long a run holds its customer: another replica takes it over past it.
const LEASE: Duration = Duration::minutes(5);
/// A run gives up before its lease ends, recorded as a failure.
const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);
/// Items a step takes per run, each once.
const STEP_BUDGET: usize = 50;
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
}

impl Services {
    /// Runs the customers whose wake is due or whose inputs changed, up to `limit`.
    pub(crate) async fn run_billing_controller(&self, limit: i64) -> StoreResult<ControllerRun> {
        let now = common_domain::clock::now();
        let claimed = {
            let mut conn = self.store.get_conn().await?;
            BillingControllerRow::claim_due(&mut conn, now, now + LEASE, limit.max(1))
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        let customers = claimed.len();
        let failed = futures::stream::iter(claimed)
            .map(|row| self.run_customer(row))
            .buffer_unordered(PARALLEL_CUSTOMERS)
            .filter(|ok| futures::future::ready(!ok))
            .count()
            .await;
        Ok(ControllerRun { customers, failed })
    }

    /// Runs one customer's due steps, then records its next wake. False when it failed.
    async fn run_customer(&self, row: BillingControllerRow) -> bool {
        let (tenant_id, customer_id) = (row.tenant_id, row.customer_id);
        let started = common_domain::clock::now();
        let steps = tokio::time::timeout(
            RUN_TIMEOUT,
            self.customer_steps(tenant_id, customer_id, started),
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
                        .map(|wake| wake.map(|at| at.max(common_domain::clock::now() + RECHECK)))
                };
                match wake {
                    Ok(wake) => {
                        let done = async {
                            let mut conn = self.store.get_conn().await?;
                            BillingControllerRow::complete(
                                &mut conn,
                                tenant_id,
                                customer_id,
                                row.dirty_since,
                                wake,
                                common_domain::clock::now(),
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)
                        }
                        .await;
                        done.map(|()| true)
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
                let now = common_domain::clock::now();
                let retry_at = now + backoff(row.attempts.saturating_add(1));
                let recorded = async {
                    let mut conn = self.store.get_conn().await?;
                    BillingControllerRow::fail(
                        &mut conn,
                        tenant_id,
                        customer_id,
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
                false
            }
        }
    }

    /// The customer's due steps, in order; true when a step took work, which may have made more
    /// due (the customer then runs again at once).
    async fn customer_steps(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let customer = (tenant_id, customer_id);
        // A change dated in the period applies before its renewal, as the renewal applies a
        // same-day change itself; what the renewal makes due (finalize, a retry) runs after it.
        let mut more = self.due_events_of(customer).await?;
        more |= self.due_cycles_of(customer).await?;
        more |= self.due_events_of(customer).await?;
        // Credit, endings and drafts see the customer once every due transition is applied: a
        // subscription ending is ended before its credit lapses as an expiry.
        if more {
            return Ok(true);
        }
        self.credit_due_of(customer, now).await?;
        self.settle_missed_endings_of(Some(customer_id), STEP_BUDGET as i64)
            .await?;
        more |= self.refresh_drafts_of(customer).await?;
        Ok(more)
    }

    /// Each due event once; true when it took any (what they made due runs next, at once).
    async fn due_events_of(&self, customer: (TenantId, CustomerId)) -> StoreResult<bool> {
        let events = {
            let mut conn = self.store.get_conn().await?;
            ScheduledEventRow::find_and_claim_due_events(
                &mut conn,
                STEP_BUDGET as i64,
                Some(customer),
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        };
        let took = !events.is_empty();
        for event in events {
            let id = event.id;
            if let Err(error) = self.process_scheduled_event(event).await {
                log::error!("scheduled event {id} not processed: {error:?}");
            }
        }
        Ok(took)
    }

    /// Each due subscription advanced one transition; true when it took any (a subscription
    /// behind by several periods advances one per run, running again at once).
    async fn due_cycles_of(&self, customer: (TenantId, CustomerId)) -> StoreResult<bool> {
        let due = {
            let mut conn = self.store.get_conn().await?;
            SubscriptionRow::claim_due_subscriptions(&mut conn, STEP_BUDGET as i64, Some(customer))
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        let took = !due.is_empty();
        for (subscription_id, customer_id, tenant_id) in due {
            match self
                .process_subscription_cycle(tenant_id, customer_id, subscription_id)
                .await
            {
                Ok(Visit::Done) => {}
                // Another writer holds the customer: the subscription defers itself.
                Ok(Visit::CustomerBusy) => {}
                Err(error) => log::error!("subscription {subscription_id} not advanced: {error:?}"),
            }
        }
        Ok(took)
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
        let now = common_domain::clock::now();
        let due = {
            let mut conn = self.store.get_conn().await?;
            DueTimes::of_customer(&mut conn, tenant_id, customer_id, now)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        Ok(wake_of(&due, now))
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
                return Ok(sweep);
            };
            after = Some(last);
            let mut lost = 0;
            for (tenant_id, customer_id) in customers {
                sweep.customers += 1;
                let now = common_domain::clock::now();
                let wake = self.next_wake(tenant_id, customer_id).await?;
                let previous = {
                    let mut conn = self.store.get_conn().await?;
                    BillingControllerRow::rewake(&mut conn, tenant_id, customer_id, wake, now)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
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
    [due.transition, due.event, lapse, due.ending, due.draft]
        .into_iter()
        .flatten()
        .min()
}

/// A failing run waits longer each time, up to an hour.
fn backoff(attempts: i32) -> Duration {
    let minutes = 1_i64 << attempts.clamp(0, 6);
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
        due.transition = Some(now + Duration::minutes(5));
        assert_eq!(wake_of(&due, now), Some(now + Duration::minutes(5)));
    }

    #[test]
    fn a_failing_run_backs_off_up_to_an_hour() {
        assert_eq!(backoff(1), Duration::minutes(2));
        assert_eq!(backoff(20), Duration::minutes(60));
    }
}
