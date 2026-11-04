//! Archives the customers created by purchase-link visits that never paid, so scripted
//! or abandoned purchases don't pile up in the tenant's customer list, and forgets spent
//! proof-of-work challenges once they expired.

use std::sync::Arc;
use std::time::Duration;

use distributed_lock::LeaderElection;
use meteroid_store::Store;
use meteroid_store::repositories::purchase_links::PurchaseLinksInterface;

const INTERVAL: Duration = Duration::from_secs(3600);
const ABANDONED_AFTER: chrono::Duration = chrono::Duration::hours(24);
const BATCH_SIZE: i64 = 500;

pub async fn run_abandoned_purchase_cleanup_worker(
    store: Arc<Store>,
    elector: Arc<dyn LeaderElection>,
) {
    loop {
        let jitter = Duration::from_secs(rand::random::<u64>() % 300);
        tokio::time::sleep(INTERVAL + jitter).await;

        match elector.try_acquire().await {
            Ok(Some(guard)) => {
                cleanup(&store).await;
                guard.release().await;
            }
            Ok(None) => {}
            Err(e) => {
                log::error!("Abandoned purchase cleanup: leader-lock acquisition failed: {e}")
            }
        }
    }
}

async fn cleanup(store: &Store) {
    let older_than = (chrono::Utc::now() - ABANDONED_AFTER).naive_utc();
    let mut total = 0;
    loop {
        match store
            .cleanup_abandoned_purchases(older_than, BATCH_SIZE)
            .await
        {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if (n as i64) < BATCH_SIZE {
                    break;
                }
            }
            Err(e) => {
                log::error!("Abandoned purchase cleanup failed: {e:?}");
                break;
            }
        }
    }
    if total > 0 {
        log::info!("Archived the customers of {total} abandoned purchases");
    }
    match store.purge_spent_pow_challenges().await {
        Ok(n) if n > 0 => log::debug!("Purged {n} expired proof-of-work challenges"),
        Ok(_) => {}
        Err(e) => log::error!("Purging spent proof-of-work challenges failed: {e:?}"),
    }
}
