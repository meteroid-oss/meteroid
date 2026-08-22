//! Hosted-payment pending-intent sweeper — the lost-return backstop for
//! `PollingRequired` providers (no webhooks: the return redirect is the only
//! completion signal). A customer who pays then closes the tab has had money
//! captured while the pre-created transaction stays Pending; each sweep
//! re-runs the SAME completion routine as the return handler
//! (`Services::sweep_hosted_payment`): records a captured payment and
//! materializes/settles — never charges — and past a max age closes out
//! abandoned attempts. Return handler and sweeper are mutually idempotent.

use chrono::{DateTime, Utc};
use common_domain::ids::PaymentTransactionId;
use distributed_lock::LeaderElection;
use meteroid_store::Services;
use meteroid_store::services::HostedPaymentSweepOutcome;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// Grace before an attempt is swept (the customer may still be typing).
const AWAITING_GRACE: Duration = Duration::from_secs(15 * 60);

/// Close-out cutoff — comfortably beyond the 24h checkout-session TTL, so a
/// hosted page cannot plausibly still capture money afterwards.
const ABANDONED_MAX_AGE: Duration = Duration::from_secs(48 * 60 * 60);

/// Provider polling budget per sweep.
const BATCH_SIZE: i64 = 25;

/// Consecutive per-attempt errors before the attempt is called out as poisoned.
const POISONED_ERROR_THRESHOLD: u32 = 3;

/// With no webhook racing us, this is the recovery latency for a lost return.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Backoff between attempts to (re)acquire leadership.
const LEADER_RETRY_SLEEP: Duration = Duration::from_secs(15);

/// Single-replica provider polling via a Postgres advisory lock; completion
/// is idempotent, so a brief overlap during re-election is harmless. Shares
/// the reconciliation kill switch.
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

/// Cross-pass sweep state: a keyset cursor rotating through ALL pending
/// attempts (so old-but-alive attempts never starve newer ones), plus
/// consecutive-error counts to surface poisoned attempts. Purely in-memory:
/// a restart only restarts the rotation (every operation is idempotent).
#[derive(Debug, Default)]
struct SweepRotation {
    cursor: Option<(DateTime<Utc>, PaymentTransactionId)>,
    error_counts: HashMap<PaymentTransactionId, u32>,
    seen_since_wrap: HashSet<PaymentTransactionId>,
}

impl SweepRotation {
    /// `batch_len < limit` means the rotation reached the end — wrap to the
    /// oldest and drop error counts for attempts that no longer exist.
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
        // One erroring attempt must never abort the batch — log, count, move on.
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

    /// A full batch advances past its last row; a short batch wraps back to
    /// the oldest.
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

    /// Errors accumulate to the poison threshold, a success resets the count,
    /// and a full wrap prunes counts for attempts that vanished.
    #[test]
    fn poison_counting_and_pruning() {
        let mut rotation = SweepRotation::default();
        let poisoned = PaymentTransactionId::new();
        let vanished = PaymentTransactionId::new();

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
