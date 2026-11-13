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
//! `POLL_INTERVAL`, least recently polled first. Each row is polled again only after a tenth of
//! its age (one minute to one hour), so providers that settle only by polling (Stancer's batch
//! capture) are not hammered. A jittered sleep prevents instances from synchronising.
//!
//! Per-transaction work is delegated to
//! [`Services::reconcile_pending_transaction`], which runs the resolved
//! status through the same `consolidate_intent_and_transaction_tx` pipeline
//! webhooks use.
//!
//! A second pass does the same for REFUND rows the provider accepted but has not resolved yet,
//! through [`Services::reconcile_pending_refund`]. Providers with no refund GET answer
//! `Unsupported`; those rows are left for their webhook.
//!
//! A third pass recovers refunds whose provider call never came back at all
//! ([`Services::replay_unconfirmed_refund`]): they never left `Ready`, so they hold the credit
//! note's remainder and the payment's refundable amount hostage until something resolves them.
//!
//! A fourth pass does the same for invoice charges committed before they were sent
//! ([`Services::resume_unsent_invoice_charge`]): they stay `Pending` with no provider id and
//! block the invoice's next payment until resolved.

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

/// The longest any provider's `refund_replay_window` runs; each row is then held to its own
/// connector's window before it is replayed.
const REFUND_REPLAY_WINDOW: Duration = Duration::from_secs(12 * 60 * 60);

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
    let now = common_domain::clock::now();
    let threshold = now - chrono::Duration::from_std(PENDING_AGE_THRESHOLD).unwrap();

    let rows = services
        .claim_pending_payment_transactions(threshold, BATCH_SIZE)
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
                tracing::warn!(
                    tenant_id = %row.tenant_id,
                    "Reconcile failed for transaction {} (created {}): {e:?}",
                    row.id,
                    row.created_at
                );
            }
        }
    }

    let refunds = services
        .claim_pending_refund_transactions(threshold, BATCH_SIZE)
        .await
        .map_err(|err| Box::new(err.into_error()) as Box<dyn std::error::Error + Send + Sync>)?;

    for row in refunds {
        let result = services
            .reconcile_pending_refund(row.id, row.tenant_id)
            .await;
        match result {
            Ok(()) => count += 1,
            Err(e) => {
                tracing::warn!(
                    tenant_id = %row.tenant_id,
                    "Reconcile failed for refund {} (created {}): {e:?}",
                    row.id,
                    row.created_at
                );
            }
        }
    }

    // Refunds whose provider call never came back: re-issued on the same idempotency key, so a
    // provider that honours it hands back the original refund instead of moving money again.
    let replay_floor = now - chrono::Duration::from_std(REFUND_REPLAY_WINDOW).unwrap();
    let unconfirmed = services
        .list_unconfirmed_refund_transactions(threshold, replay_floor, BATCH_SIZE)
        .await
        .map_err(|err| Box::new(err.into_error()) as Box<dyn std::error::Error + Send + Sync>)?;

    for row in unconfirmed {
        match services
            .replay_unconfirmed_refund(row.id, row.tenant_id)
            .await
        {
            Ok(()) => count += 1,
            Err(e) => {
                tracing::warn!(
                    tenant_id = %row.tenant_id,
                    "Replay failed for unconfirmed refund {} (created {}): {e:?}",
                    row.id,
                    row.created_at
                );
            }
        }
    }

    // Invoice charges committed before sending whose answer never came back: re-sent under the
    // key they were committed with, or failed once the provider may have forgotten it.
    let unsent = services
        .claim_unsent_invoice_charges(threshold, BATCH_SIZE)
        .await
        .map_err(|err| Box::new(err.into_error()) as Box<dyn std::error::Error + Send + Sync>)?;

    for row in unsent {
        match services
            .resume_unsent_invoice_charge(row.id, row.tenant_id)
            .await
        {
            Ok(()) => count += 1,
            Err(e) => {
                log::warn!(
                    "Resume failed for unsent charge {} (created {}): {e:?}",
                    row.id,
                    row.created_at
                );
            }
        }
    }

    Ok(count)
}
