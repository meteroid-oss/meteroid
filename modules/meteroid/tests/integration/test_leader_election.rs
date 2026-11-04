//! Session-scoped leader locks: whether a guard is released or merely dropped, another replica
//! must be able to take the lock over.

use std::time::Duration;

use distributed_lock::{LeaderElection, LeaderGuard};
use meteroid_store::leader_lock::LeaderLock;
use meteroid_store::leader_lock::PgLeaderElection;
use meteroid_store::store::{PgConfig, diesel_make_pg_pool};

use crate::meteroid_it;

// Each test gets its own database, and advisory locks are per-database, so no worker contends.
const KEY: LeaderLock = LeaderLock::AnalyticsProjection;

fn replica(url: &str) -> PgLeaderElection {
    let pool = diesel_make_pg_pool(PgConfig::new(url.to_string())).expect("pool");
    PgLeaderElection::new(pool, KEY)
}

/// Server-side, the lock goes away once Postgres notices the closed session, so poll briefly.
async fn acquire_within(elector: &PgLeaderElection, timeout: Duration) -> Box<dyn LeaderGuard> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(guard) = elector.try_acquire().await.expect("try_acquire") {
            return guard;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "lock not acquired within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_released_guard_hands_over_leadership() {
    crate::helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let (a, b) = (replica(&db.url), replica(&db.url));

    let guard = a.try_acquire().await.unwrap().expect("a leads");
    assert!(b.try_acquire().await.unwrap().is_none());

    guard.release().await;
    let guard = b
        .try_acquire()
        .await
        .unwrap()
        .expect("b leads after release");
    assert!(a.try_acquire().await.unwrap().is_none());
    guard.release().await;
}

#[tokio::test]
async fn test_dropped_guard_hands_over_leadership() {
    crate::helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let (a, b) = (replica(&db.url), replica(&db.url));

    let guard = a.try_acquire().await.unwrap().expect("a leads");
    drop(guard);

    let guard = acquire_within(&b, Duration::from_secs(5)).await;
    guard.release().await;
    acquire_within(&a, Duration::from_secs(5))
        .await
        .release()
        .await;
}
