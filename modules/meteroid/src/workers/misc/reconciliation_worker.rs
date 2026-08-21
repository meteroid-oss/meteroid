//! Reconciliation worker. Periodically polls payment providers for the
//! authoritative status of transactions stuck in `Pending` longer than
//! expected — covers the case where the provider's webhook for a successful
//! charge was lost in flight.
//!
//! Why this exists. Stripe / GoCardless / Adyen all retry webhook delivery
//! but the SLA is best-effort. Without reconciliation, a single dropped
//! webhook leaves an invoice marked unpaid in our system even though the
//! customer's bank already moved the funds — a customer-facing dispute
//! waiting to happen.
//!
//! Cadence. We sweep up to `BATCH_SIZE` stuck transactions every
//! `POLL_INTERVAL`. The interval is tuned to balance "find lost webhooks
//! quickly" against "don't hammer the provider's API". A jittered sleep
//! prevents instances from synchronising.
//!
//! Per-transaction work is delegated to
//! [`Services::reconcile_pending_transaction`], which runs the resolved
//! status through the same `consolidate_intent_and_transaction_tx` pipeline
//! webhooks use.

use distributed_lock::LeaderElection;
use meteroid_store::Services;
use meteroid_store::Store;
use std::sync::Arc;
use std::time::Duration;

/// How long a transaction must sit in `Pending` before we'll poll the
/// provider for it. Should be longer than typical webhook delivery latency
/// to avoid wasted calls when the webhook is merely slow.
const PENDING_AGE_THRESHOLD: Duration = Duration::from_secs(10 * 60);

/// How many stale transactions we process per sweep. Provider APIs are
/// rate-limited; keep this conservative.
const BATCH_SIZE: i64 = 50;

/// How often the worker sweeps.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Backoff between attempts to (re)acquire leadership.
const LEADER_RETRY_SLEEP: Duration = Duration::from_secs(15);

/// Poll providers from a SINGLE replica only. Every process used to run this
/// loop, so a fleet of N replicas issued N× the provider polling against a
/// rate-limited API. Leadership is held via a Postgres advisory lock; the
/// consolidation pipeline is idempotent, so a brief overlap during re-election
/// is harmless. `enabled` is a kill switch (mirrors sandbox_maintenance_enabled).
pub async fn run_reconciliation_worker(
    store: Arc<Store>,
    services: Arc<Services>,
    elector: Arc<dyn LeaderElection>,
    enabled: bool,
) {
    if !enabled {
        log::info!("Reconciliation worker disabled (RECONCILIATION_ENABLED=false)");
        return;
    }
    log::info!("Reconciliation worker started");

    loop {
        let mut guard = loop {
            match elector.try_acquire().await {
                Ok(Some(guard)) => break guard,
                Ok(None) => tokio::time::sleep(LEADER_RETRY_SLEEP).await,
                Err(e) => {
                    log::error!("Reconciliation worker: leader-lock acquisition failed: {e}");
                    tokio::time::sleep(LEADER_RETRY_SLEEP).await;
                }
            }
        };
        log::info!("Reconciliation worker: leadership acquired");

        loop {
            let jitter = Duration::from_millis(rand::random::<u64>() % 10_000);
            match sweep(&store, &services).await {
                Ok(count) if count > 0 => {
                    log::info!("Reconciliation sweep reconciled {count} transactions");
                }
                Ok(_) => {
                    log::debug!("Reconciliation sweep found nothing to do");
                }
                Err(e) => {
                    log::error!("Reconciliation sweep failed: {e:?}");
                }
            }
            tokio::time::sleep(POLL_INTERVAL + jitter).await;

            // Lock connection dropped => leadership lost; re-elect.
            if !guard.is_held().await {
                log::warn!("Reconciliation worker: lost leadership; re-electing");
                break;
            }
        }

        guard.release().await;
    }
}

async fn sweep(
    _store: &Arc<Store>,
    services: &Arc<Services>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    // Filter at the DB layer using `created_at` — the partial index
    // `idx_payment_transaction_pending_created_at` keeps this fast even
    // with millions of historical rows. Fresh-Pending transactions whose
    // webhook is still in flight are excluded, so the worker doesn't burn
    // provider rate limit polling them.
    let now = chrono::Utc::now();
    let threshold = now - chrono::Duration::from_std(PENDING_AGE_THRESHOLD).unwrap();

    let rows = services
        .list_pending_payment_transactions(threshold, BATCH_SIZE)
        .await
        .map_err(|err| Box::new(err.into_error()) as Box<dyn std::error::Error + Send + Sync>)?;

    let mut count = 0usize;
    for row in rows {
        let result = services
            .reconcile_pending_transaction(row.id, row.tenant_id)
            .await;
        match result {
            Ok(()) => count += 1,
            Err(e) => {
                log::warn!(
                    "Reconcile failed for transaction {} (created {}): {e:?}",
                    row.id,
                    row.created_at
                );
            }
        }
    }

    Ok(count)
}
