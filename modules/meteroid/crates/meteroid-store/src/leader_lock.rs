//! Fixed advisory-lock keys for singleton leader election. `#[repr(i64)]` with
//! explicit discriminants makes a duplicate key a compile error, and
//! `PgLeaderElection` only accepts this enum, so no worker can pick its own number.

use crate::store::{PgConn, PgPool};
use diesel_models::query::advisory_lock;
use distributed_lock::{LeaderElection, LeaderError, LeaderGuard};

#[repr(i64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeaderLock {
    /// Analytics projection worker.
    AnalyticsProjection = 92_000_001,
    /// Alert evaluation worker.
    AlertEvaluation = 92_000_002,
    /// Weekly tenant-deletion cleanup worker.
    TenantDeletionCleanup = 92_000_003,
    /// Daily tenant-lifecycle worker.
    TenantLifecycle = 92_000_004,
    /// Daily VAT revalidation enqueuer.
    VatRevalidation = 92_000_005,
    /// Organization activation-milestones derivation worker.
    DogfoodingMilestones = 92_000_006,
    /// Hourly stale-draft invoice refresh worker.
    DraftRefresh = 92_000_007,
    /// Payment reconciliation worker (provider polling).
    Reconciliation = 92_000_008,
    /// Hosted-checkout pending-intent sweeper (provider polling).
    HostedPaymentSweep = 92_000_009,
    /// Customer activation milestone derivation worker.
    CustomerActivation = 92_000_010,
    /// One-shot webhook subscription resync at startup.
    WebhookSubscriptionSync = 92_000_011,
    /// Payment reminder sweeper (due scan + enrolment).
    PaymentReminderSweep = 92_000_012,
    /// Daily target snapshot worker.
    TargetSnapshot = 92_000_013,
    /// Live credit and usage-monitor recompute worker.
    CreditRecompute = 92_000_015,
    /// Credit input change feed partition maintenance.
    CreditInputFeed = 92_000_016,
    /// Hourly online license refresh.
    LicenseRefresh = 92_000_017,
    /// Serializes license token rotations across replicas.
    LicenseRotation = 92_000_018,
    /// Issues subscription licenses and revokes those of ended subscriptions.
    LicenseReconcile = 92_000_019,
    /// Hourly cleanup of abandoned purchase-link purchases.
    AbandonedPurchaseCleanup = 92_000_020,
    /// Hourly check whether the daily anonymous instance report is due.
    TelemetryReport = 92_000_021,
}

impl LeaderLock {
    pub const fn key(self) -> i64 {
        self as i64
    }
}

/// Postgres leader election via session-scoped `pg_try_advisory_lock`; the guard owns the locking connection.
pub struct PgLeaderElection {
    pool: PgPool,
    key: i64,
}

impl PgLeaderElection {
    pub fn new(pool: PgPool, lock: LeaderLock) -> Self {
        Self {
            pool,
            key: lock.key(),
        }
    }
}

struct PgLeaderGuard {
    conn: Option<PgConn>,
    key: i64,
}

impl Drop for PgLeaderGuard {
    /// A guard dropped without `release` must not hand its still-locked session back to the pool:
    /// closing the connection ends the session, and Postgres frees the lock with it.
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            drop(PgConn::take(conn));
        }
    }
}

#[async_trait::async_trait]
impl LeaderElection for PgLeaderElection {
    async fn try_acquire(&self) -> Result<Option<Box<dyn LeaderGuard>>, LeaderError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| LeaderError::Election(format!("{e:?}")))?;
        let acquired = advisory_lock::try_advisory_lock(&mut conn, self.key)
            .await
            .map_err(|e| LeaderError::Election(format!("{e:?}")))?;
        if acquired {
            Ok(Some(Box::new(PgLeaderGuard {
                conn: Some(conn),
                key: self.key,
            })))
        } else {
            Ok(None)
        }
    }
}

#[async_trait::async_trait]
impl LeaderGuard for PgLeaderGuard {
    async fn is_held(&mut self) -> bool {
        match self.conn.as_mut() {
            Some(conn) => advisory_lock::connection_alive(conn).await,
            None => false,
        }
    }

    async fn release(mut self: Box<Self>) {
        if let Some(mut conn) = self.conn.take()
            && !matches!(
                advisory_lock::advisory_unlock(&mut conn, self.key).await,
                Ok(true)
            )
        {
            drop(PgConn::take(conn));
        }
    }
}
