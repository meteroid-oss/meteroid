use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::sandbox::SandboxInterface;
use meteroid_store::{Services, Store};
use std::sync::Arc;
use std::time::Duration;

const SWEEP_INTERVAL: Duration = Duration::from_secs(300);
const BATCH: i64 = 200;

/// Settles and records the lapse of expired customer credit, and reconciles reversals a pre-ledger
/// instance skipped (removed once none can run). Each pass pages through every due customer. A late
/// pack whose rest was forgiven is credited back first, should its note's event have been missed.
pub async fn run_customer_credit_worker(store: Arc<Store>, services: Arc<Services>) {
    loop {
        let jitter = Duration::from_secs(rand::random::<u64>() % 30);
        if let Err(err) = services.credit_back_forgiven_late_packs_due(BATCH).await {
            log::error!("forgiven late pack credit-back failed: {err:?}");
        }
        if let Err(err) = store.expire_customer_credit(BATCH).await {
            log::error!("customer credit expiry sweep failed: {err:?}");
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

const PLAN_CHANGE_INTERVAL: Duration = Duration::from_secs(600);

/// Settles what plan changes ended on invoices issued without it (by an earlier release during a
/// deploy). Idempotent: a change already settled is left as is.
pub async fn run_plan_change_settlement_worker(services: Arc<Services>) {
    loop {
        match services.settle_missed_plan_change_endings(BATCH).await {
            Ok(0) => {}
            Ok(issued) => log::warn!("settled {issued} credit note(s) a plan change had missed"),
            Err(err) => log::error!("plan change settlement sweep failed: {err:?}"),
        }
        let jitter = Duration::from_secs(rand::random::<u64>() % 60);
        tokio::time::sleep(PLAN_CHANGE_INTERVAL + jitter).await;
    }
}
