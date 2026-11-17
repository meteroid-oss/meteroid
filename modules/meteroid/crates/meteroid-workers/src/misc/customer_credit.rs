use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::sandbox::SandboxInterface;
use meteroid_store::{Services, Store};
use std::sync::Arc;
use std::time::Duration;

const SWEEP_INTERVAL: Duration = Duration::from_secs(300);
const BATCH: i64 = 200;

/// Repairs credit the expiry missed (ended subscriptions, orphaned settlements) and reconciles
/// reversals a pre-ledger instance skipped (removed once none can run). The expiry itself, and a
/// forgiven late pack's credit-back, are each customer's billing controller's.
pub async fn run_customer_credit_worker(store: Arc<Store>) {
    loop {
        let jitter = Duration::from_secs(rand::random::<u64>() % 30);
        // The expiry itself is each customer's billing controller's.
        if let Err(err) = store.repair_customer_credit(BATCH).await {
            log::error!("customer credit repair failed: {err:?}");
        }
        if let Err(err) = store.reconcile_reversed_credit_purchases(BATCH).await {
            log::error!("reversed credit purchase reconciliation failed: {err:?}");
        }
        // A seed left running holds its sandbox's credit automation until it is failed.
        match store.reap_interrupted_sandbox_seeds().await {
            Ok(reaped) if !reaped.is_empty() => {
                log::warn!(
                    "failed {} interrupted sandbox seed(s): {reaped:?}",
                    reaped.len()
                )
            }
            Ok(_) => {}
            Err(err) => log::error!("interrupted sandbox seed reaping failed: {err:?}"),
        }
        tokio::time::sleep(SWEEP_INTERVAL + jitter).await;
    }
}

const PURCHASE_INTERVAL: Duration = Duration::from_secs(30);

/// Resumes top-ups whose provider answer was lost, automatic ones included: settling what is in
/// flight is core's, arming automatic top-ups is the usage controls'. Replicas share the work:
/// stuck attempts are claimed with `SKIP LOCKED`.
pub async fn run_wallet_purchase_worker(services: Arc<Services>) {
    loop {
        if let Err(err) = services.run_wallet_purchase_executor().await {
            log::error!("wallet purchase executor failed: {err:?}");
        }
        let jitter = Duration::from_secs(rand::random::<u64>() % 10);
        tokio::time::sleep(PURCHASE_INTERVAL + jitter).await;
    }
}
