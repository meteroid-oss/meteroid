use crate::utils::{backoff_duration, sleep_with_jitter};
use meteroid_store::Services;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Past each customer's own run timeout (inside its lease), a pass that hangs.
const TIMEOUT: Duration = Duration::from_secs(600);
const IDLE_POLL: Duration = Duration::from_secs(2);
const CLAIM: i64 = 16;
/// Scheduled events a crashed run left processing go back to pending this often.
const STUCK_EVENTS_EVERY: Duration = Duration::from_secs(60);

/// Runs the customers whose billing is due, as long as there are some (every replica: the claim
/// leases each customer to one).
pub async fn run_worker(services: Arc<Services>) {
    // A pending plan change or amendment the release before this one scheduled becomes facts
    // before anything bills: the next release, which drops scheduled_event, finds none.
    match services.migrate_pending_changes().await {
        Ok(None) => log::info!("pending changes migrated by another replica"),
        Ok(Some(migration)) if migration.unresolved.is_empty() => {
            if migration.written > 0 {
                log::info!("{} pending changes written as facts", migration.written);
            }
        }
        Ok(Some(migration)) => log::error!(
            "{} pending changes cannot be written as facts: {:?}",
            migration.unresolved.len(),
            migration.unresolved
        ),
        Err(err) => log::error!("pending changes not written as facts: {err:?}"),
    }
    let mut consecutive_errors: u32 = 0;
    let mut stuck_checked = Instant::now();
    loop {
        if stuck_checked.elapsed() >= STUCK_EVENTS_EVERY {
            stuck_checked = Instant::now();
            if let Err(err) = services.cleanup_timeout_scheduled_events().await {
                log::error!("stuck scheduled events not released: {err:?}");
            }
        }
        match tokio::time::timeout(TIMEOUT, services.run_billing_controller(CLAIM)).await {
            Ok(Ok(run)) => {
                consecutive_errors = 0;
                if run.failed > 0 {
                    log::warn!(
                        "billing controller: {} of {} customers failed",
                        run.failed,
                        run.customers
                    );
                }
                if run.customers == 0 {
                    sleep_with_jitter(IDLE_POLL).await;
                }
            }
            Ok(Err(err)) => {
                consecutive_errors += 1;
                log::error!("billing controller failed: {err:?}");
                sleep_with_jitter(backoff_duration(consecutive_errors)).await;
            }
            Err(_) => {
                consecutive_errors += 1;
                log::error!(
                    "billing controller timed out after {} seconds",
                    TIMEOUT.as_secs()
                );
                sleep_with_jitter(backoff_duration(consecutive_errors)).await;
            }
        }
    }
}
