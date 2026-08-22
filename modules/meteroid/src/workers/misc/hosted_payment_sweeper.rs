//! Hosted-payment pending-intent sweeper — the lost-return backstop for
//! providers whose hosted setup completes by polling (capability
//! `HostedSetupCompletion::PollingRequired`), unified over hosted CHECKOUTS
//! and hosted INVOICE payments.
//!
//! Why this exists. Such a provider's hosted flow captures the REAL amount
//! in-flow on the hosted page and has NO webhook mechanism: the only
//! completion signal is the customer's return redirect. A customer who pays
//! and then closes the tab (or loses the redirect) has had money captured
//! while the pre-created payment transaction stays Pending, the subscription
//! never activates / the invoice never closes. Webhook-backed providers
//! (GoCardless) get this backstop from their webhook
//! (`billing_requests.fulfilled`) and never persist a sweepable intent id;
//! for polling providers this worker is it.
//!
//! Each sweep re-runs the SAME completion routine the return handler uses
//! (`Services::sweep_hosted_payment` →
//! `complete_hosted_setup_with_attempts`): it reads the intent, records a
//! captured payment and materializes the checkout / settles the invoice —
//! never charges — and past a max age closes out abandoned attempts (cancels
//! the intent + pending transaction, expires a checkout session). Return
//! handler and sweeper are mutually idempotent, so both may run.

use chrono::{DateTime, Utc};
use common_domain::ids::PaymentTransactionId;
use distributed_lock::LeaderElection;
use meteroid_store::Services;
use meteroid_store::services::HostedPaymentSweepOutcome;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// How long after initiation before an attempt is swept. The return redirect
/// normally completes within seconds; the grace avoids polling intents whose
/// customer is still typing on the hosted page.
const AWAITING_GRACE: Duration = Duration::from_secs(15 * 60);

/// Past this age with no captured payment the attempt is closed out (pending
/// transaction cancelled, checkout session expired). Comfortably beyond the
/// 24h checkout-session TTL, so a hosted page cannot plausibly still capture
/// money afterwards.
const ABANDONED_MAX_AGE: Duration = Duration::from_secs(48 * 60 * 60);

/// Provider polling budget per sweep.
const BATCH_SIZE: i64 = 25;

/// Consecutive per-attempt completion errors before the attempt is called out
/// as poisoned (log-loud, every subsequent pass). The rotation cursor already
/// keeps a poisoned attempt from blocking the queue; this makes it impossible
/// to miss in the logs.
const POISONED_ERROR_THRESHOLD: u32 = 3;

/// How often the worker sweeps. There is no webhook racing us, so this is the
/// recovery latency for a lost return.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Backoff between attempts to (re)acquire leadership.
const LEADER_RETRY_SLEEP: Duration = Duration::from_secs(15);

/// Single-replica provider polling, like the reconciliation worker: leadership
/// via a Postgres advisory lock; completion is idempotent, so a brief overlap
/// during re-election is harmless. Shares the reconciliation kill switch (both
/// are provider-polling settlement backstops).
pub async fn run_hosted_payment_sweeper(
    services: Arc<Services>,
    elector: Arc<dyn LeaderElection>,
    enabled: bool,
) {
    if !enabled {
        log::info!("Hosted payment sweeper disabled (RECONCILIATION_ENABLED=false)");
        return;
    }
    log::info!("Hosted payment sweeper started");

    let mut rotation = SweepRotation::default();

    loop {
        let mut guard = loop {
            match elector.try_acquire().await {
                Ok(Some(guard)) => break guard,
                Ok(None) => tokio::time::sleep(LEADER_RETRY_SLEEP).await,
                Err(e) => {
                    log::error!("Hosted payment sweeper: leader-lock acquisition failed: {e}");
                    tokio::time::sleep(LEADER_RETRY_SLEEP).await;
                }
            }
        };
        log::info!("Hosted payment sweeper: leadership acquired");

        loop {
            let jitter = Duration::from_millis(rand::random::<u64>() % 10_000);
            match sweep(&services, &mut rotation).await {
                Ok((completed, expired)) if completed > 0 || expired > 0 => {
                    log::info!(
                        "Hosted payment sweep completed {completed} attempts, closed out {expired} abandoned attempts"
                    );
                }
                Ok(_) => {
                    log::debug!("Hosted payment sweep found nothing to do");
                }
                Err(e) => {
                    log::error!("Hosted payment sweep failed: {e:?}");
                }
            }
            tokio::time::sleep(POLL_INTERVAL + jitter).await;

            if !guard.is_held().await {
                log::warn!("Hosted payment sweeper: lost leadership; re-electing");
                break;
            }
        }

        guard.release().await;
    }
}

/// Cross-pass sweep state: a keyset cursor that rotates through ALL pending
/// attempts across passes (so a wall of old-but-alive attempts can never
/// starve recovery of a newer paid-but-lost capture behind the `LIMIT`
/// window), plus per-attempt consecutive-error counts so a poisoned attempt
/// (e.g. a disconnected connector) is surfaced log-loud instead of silently
/// retried forever. Purely in-memory: a restart only restarts the rotation
/// from the oldest attempt, which is safe (every operation is idempotent).
#[derive(Debug, Default)]
struct SweepRotation {
    cursor: Option<(DateTime<Utc>, PaymentTransactionId)>,
    error_counts: HashMap<PaymentTransactionId, u32>,
    seen_since_wrap: HashSet<PaymentTransactionId>,
}

impl SweepRotation {
    /// Advance the cursor past a processed batch; `batch_len < limit` means
    /// the rotation reached the end — wrap to the oldest and drop error
    /// counts for attempts that no longer exist (completed/closed out).
    fn advance(
        &mut self,
        last_key: Option<(DateTime<Utc>, PaymentTransactionId)>,
        batch_len: usize,
        limit: usize,
    ) {
        if batch_len < limit || last_key.is_none() {
            self.cursor = None;
            self.error_counts
                .retain(|id, _| self.seen_since_wrap.contains(id));
            self.seen_since_wrap.clear();
        } else {
            self.cursor = last_key;
        }
    }

    /// Record a completion error; returns the consecutive-error count.
    fn record_error(&mut self, id: PaymentTransactionId) -> u32 {
        self.seen_since_wrap.insert(id);
        let count = self.error_counts.entry(id).or_insert(0);
        *count += 1;
        *count
    }

    /// Record a successful (any non-Err) sweep of an attempt.
    fn record_ok(&mut self, id: PaymentTransactionId) {
        self.seen_since_wrap.insert(id);
        self.error_counts.remove(&id);
    }
}

async fn sweep(
    services: &Arc<Services>,
    rotation: &mut SweepRotation,
) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
    let now = chrono::Utc::now();
    let older_than = now - chrono::Duration::from_std(AWAITING_GRACE).unwrap();
    let abandoned_before = now - chrono::Duration::from_std(ABANDONED_MAX_AGE).unwrap();

    let items = services
        .list_pending_hosted_payments(older_than, rotation.cursor, BATCH_SIZE)
        .await
        .map_err(|err| Box::new(err.into_error()) as Box<dyn std::error::Error + Send + Sync>)?;

    let batch_len = items.len();
    let last_key = items
        .last()
        .map(|item| (item.created_at, item.transaction_id));

    let mut completed = 0usize;
    let mut expired = 0usize;
    for item in items {
        // Per-attempt isolation: one erroring attempt must never abort the
        // batch or wedge the sweeper — log, count, and move on.
        match services.sweep_hosted_payment(&item, abandoned_before).await {
            Ok(HostedPaymentSweepOutcome::Completed) => {
                log::info!(
                    "Hosted payment sweep recovered lost-return transaction {} (intent {}, \
                     checkout {:?}, invoice {:?})",
                    item.transaction_id,
                    item.intent_id,
                    item.checkout_session_id,
                    item.invoice_id
                );
                rotation.record_ok(item.transaction_id);
                completed += 1;
            }
            Ok(HostedPaymentSweepOutcome::Expired) => {
                rotation.record_ok(item.transaction_id);
                expired += 1;
            }
            Ok(HostedPaymentSweepOutcome::Declined | HostedPaymentSweepOutcome::StillPending) => {
                rotation.record_ok(item.transaction_id);
            }
            Err(e) => {
                let errors = rotation.record_error(item.transaction_id);
                if errors >= POISONED_ERROR_THRESHOLD {
                    log::error!(
                        "Hosted payment sweep: transaction {} (intent {}, created {}) has failed \
                         {errors} consecutive sweeps and needs attention: {e:?}",
                        item.transaction_id,
                        item.intent_id,
                        item.created_at
                    );
                } else {
                    log::warn!(
                        "Hosted payment sweep failed for transaction {} (intent {}): {e:?}",
                        item.transaction_id,
                        item.intent_id
                    );
                }
            }
        }
    }

    rotation.advance(last_key, batch_len, BATCH_SIZE as usize);

    Ok((completed, expired))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;

    fn key(id: PaymentTransactionId) -> (DateTime<Utc>, PaymentTransactionId) {
        (Utc::now(), id)
    }

    /// The cursor rotates: a full batch advances past its last row (the next
    /// pass scans DIFFERENT attempts instead of re-hitting the same oldest
    /// window), and a short batch wraps back to the oldest.
    #[test]
    fn rotation_cursor_advances_and_wraps() {
        let mut rotation = SweepRotation::default();
        assert!(rotation.cursor.is_none());

        let a = PaymentTransactionId::new();
        let k = key(a);
        // Full batch → advance.
        rotation.advance(Some(k), 25, 25);
        assert_eq!(rotation.cursor, Some(k));

        // Short batch → wrap to the start.
        rotation.advance(Some(key(PaymentTransactionId::new())), 3, 25);
        assert!(rotation.cursor.is_none());

        // Empty batch → wrap too.
        rotation.advance(None, 0, 25);
        assert!(rotation.cursor.is_none());
    }

    /// Consecutive errors accumulate to the poison threshold, a success
    /// resets the count, and a full wrap prunes counts for attempts that
    /// vanished (completed by the return handler between passes).
    #[test]
    fn poison_counting_and_pruning() {
        let mut rotation = SweepRotation::default();
        let poisoned = PaymentTransactionId::new();
        let vanished = PaymentTransactionId::new();

        // Consecutive errors reach the poison threshold.
        let mut last = 0;
        for _ in 0..POISONED_ERROR_THRESHOLD {
            last = rotation.record_error(poisoned);
        }
        assert_eq!(last, POISONED_ERROR_THRESHOLD);

        // An error then a success → count fully resets.
        assert_eq!(rotation.record_error(vanished), 1);
        rotation.record_ok(vanished);
        assert_eq!(rotation.record_error(vanished), 1);

        // Wrap: only attempts seen since the last wrap keep their counts.
        rotation.error_counts.insert(PaymentTransactionId::new(), 5); // never seen again
        let before = rotation.error_counts.len();
        assert_eq!(before, 3);
        rotation.advance(None, 0, 25);
        // `poisoned` and `vanished` were seen this rotation; the orphan count
        // was pruned.
        assert_eq!(rotation.error_counts.len(), 2);
        assert!(rotation.error_counts.contains_key(&poisoned));
        assert!(rotation.error_counts.contains_key(&vanished));
        // After the wrap the seen-set restarts.
        assert!(rotation.seen_since_wrap.is_empty());
    }
}
