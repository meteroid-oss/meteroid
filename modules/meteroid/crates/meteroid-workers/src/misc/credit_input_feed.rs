//! Keeps the credit input change feed partitioned ahead and trimmed behind (live-recompute v2).

use std::str::FromStr;
use std::sync::{Arc, LazyLock};

use common_logging::GLOBAL_METER;
use distributed_lock::LeaderElection;
use meteroid_store::Store;
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge};

use meteroid_app::config::ParsedCron;

// Hourly: a missed run costs an hour of the 14 days kept ahead.
const SCHEDULE: &str = "35 * * * *";

static PARTITIONS_AHEAD: LazyLock<Gauge<i64>> = LazyLock::new(|| {
    GLOBAL_METER
        .i64_gauge("credit_input_feed_partitions_ahead")
        .with_description("Consecutive days after today with a feed partition; alert below 7")
        .build()
});

static DEFAULT_ROWS: LazyLock<Gauge<u64>> = LazyLock::new(|| {
    GLOBAL_METER
        .u64_gauge("credit_input_feed_default_rows")
        .with_description("Feed rows in the DEFAULT partition: a day had no partition")
        .build()
});

static FAILURES: LazyLock<Counter<u64>> = LazyLock::new(|| {
    GLOBAL_METER
        .u64_counter("credit_input_feed_maintenance_failures")
        .with_description("Feed maintenance steps that failed, by step; the next pass retries them")
        .build()
});

pub async fn run_credit_input_feed_worker(store: Arc<Store>, elector: Arc<dyn LeaderElection>) {
    let schedule = ParsedCron::from_str(SCHEDULE).expect("valid credit input feed schedule");
    super::run_scheduled("credit input feed", schedule, elector, move || {
        let store = store.clone();
        async move {
            let today = common_domain::clock::now().date_naive();
            let outcome = store.maintain_credit_input_feed(today).await;
            PARTITIONS_AHEAD.record(outcome.days_ahead, &[]);
            if let Some(rows) = outcome.default_rows {
                DEFAULT_ROWS.record(u64::try_from(rows).unwrap_or(0), &[]);
            }
            for (step, _) in &outcome.failures {
                FAILURES.add(1, &[KeyValue::new("step", step.as_str())]);
            }
            if !outcome.created.is_empty() || !outcome.dropped.is_empty() {
                log::info!(
                    "credit input feed: created {:?}, dropped {:?}",
                    outcome.created,
                    outcome.dropped
                );
            }
            if !outcome.skipped.is_empty() {
                log::warn!(
                    "credit input feed: days {:?} stay in the DEFAULT partition, which holds their rows",
                    outcome.skipped
                );
            }
            Ok(())
        }
    })
    .await;
}
