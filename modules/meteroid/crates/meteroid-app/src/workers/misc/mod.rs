use std::future::Future;
use std::sync::Arc;

use croner::Cron;
use distributed_lock::LeaderElection;

use crate::config::ParsedCron;

/// Sleep until the next occurrence of a (UTC, wall-clock) cron schedule. Falls
/// back to a 1-minute sleep if the schedule somehow yields no future time.
pub async fn sleep_until_next(cron: &Cron) {
    let now = common_domain::clock::wall();
    let dur = cron
        .find_next_occurrence(&now, false)
        .ok()
        .and_then(|next| (next - now).to_std().ok())
        .unwrap_or(std::time::Duration::from_secs(60));
    tokio::time::sleep(dur).await;
}

/// Run `job` on a wall-clock cron `schedule`, but only on the scheduler replica
/// that holds the `elector` advisory lock. Errors are logged (prefixed with
/// `name`) and never break the loop.
pub async fn run_scheduled<F, Fut>(
    name: &'static str,
    cron: ParsedCron,
    elector: Arc<dyn LeaderElection>,
    job: F,
) where
    F: Fn() -> Fut + Send,
    Fut: Future<Output = meteroid_store::StoreResult<()>> + Send,
{
    loop {
        sleep_until_next(&cron).await;
        match elector.try_acquire().await {
            Ok(Some(guard)) => {
                if let Err(e) = job().await {
                    log::warn!("{name} worker error: {e:?}");
                }
                guard.release().await;
            }
            Ok(None) => {}
            Err(e) => log::error!("{name}: leader-lock acquisition failed: {e}"),
        }
    }
}
