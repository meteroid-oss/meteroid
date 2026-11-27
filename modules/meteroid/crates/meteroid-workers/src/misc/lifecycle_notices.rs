//! Publishes the notices of overrides ending (R-148) and of trials ending (R-158), hourly.
use meteroid_store::Store;
use meteroid_store::services::{override_notices, trial_notices};
use std::sync::Arc;
use std::time::Duration;

const INTERVAL_SECS: u64 = 3600;

pub async fn run_lifecycle_notices_worker(store: Arc<Store>) {
    loop {
        let now = common_clock::now();
        match override_notices::notify_override_endings(&store, now).await {
            Ok((0, 0)) => {}
            Ok((ending, ended)) => {
                log::info!("Override notices: {ending} ending soon, {ended} ended")
            }
            Err(err) => log::error!("Override notices failed: {err:?}"),
        }
        match trial_notices::notify_trial_endings(&store, now).await {
            Ok(0) => {}
            Ok(noticed) => log::info!("Trial ending notices: {noticed} published"),
            Err(err) => log::error!("Trial ending notices failed: {err:?}"),
        }
        let jitter = Duration::from_secs(rand::random::<u64>() % 300);
        tokio::time::sleep(Duration::from_secs(INTERVAL_SECS) + jitter).await;
    }
}
