//! Payment reminder sweeper: every cron tick the leader executes the due reminder steps and
//! enrols newly eligible invoices (`Services::run_payment_reminder_sweep`), then tells customers
//! whose card expires soon (`Services::run_card_expiry_sweep`). Leader election
//! keeps the per-entity keyset scan on one replica; execution locks per invoice.

use distributed_lock::LeaderElection;
use meteroid_app::config::ParsedCron;
use meteroid_store::Services;
use std::sync::Arc;

pub async fn run_payment_reminder_sweeper(
    services: Arc<Services>,
    elector: Arc<dyn LeaderElection>,
    schedule: ParsedCron,
) {
    super::run_scheduled("payment reminder sweeper", schedule, elector, move || {
        let services = services.clone();
        async move { run_once(&services).await }
    })
    .await;
}

async fn run_once(services: &Arc<Services>) -> meteroid_store::StoreResult<()> {
    let now = common_clock::now().naive_utc();
    let stats = services.run_payment_reminder_sweep(now).await?;
    let told = services.run_card_expiry_sweep(now).await?;
    if told > 0 {
        log::info!("Card expiry sweep: {told} customers told their card expires soon");
    }
    if stats.did_work() {
        log::info!(
            "Payment reminder sweep: {} executed, {} deferred, {} completed, {} ended, \
             {} enrolled, {} not enrolled, {} quarantined, {} entities skipped{}",
            stats.executed,
            stats.deferred,
            stats.completed,
            stats.ended,
            stats.enrolled,
            stats.not_enrolled,
            stats.quarantined,
            stats.entity_errors,
            if stats.truncated {
                " (budget hit, backlog continues next run)"
            } else {
                ""
            }
        );
    }
    Ok(())
}
