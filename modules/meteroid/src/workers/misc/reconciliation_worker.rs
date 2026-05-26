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

pub async fn run_reconciliation_worker(store: Arc<Store>, services: Arc<Services>) {
    log::info!("Reconciliation worker started");
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
    let now = chrono::Utc::now().naive_utc();
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
