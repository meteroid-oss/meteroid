//! Postgres-backed single-leader election for workers that must run on exactly one
//! replica (e.g. provider polling against a rate-limited API). Leadership is a
//! session advisory lock held on a dedicated pooled connection: dropping the
//! connection releases it, so `is_held` only has to probe liveness.

use crate::store::{PgConn, PgPool};
use diesel_models::query::advisory_lock;
use distributed_lock::{LeaderElection, LeaderError, LeaderGuard};

pub struct PgLeaderElection {
    pool: PgPool,
    key: i64,
}

impl PgLeaderElection {
    pub fn new(pool: PgPool, key: i64) -> Self {
        Self { pool, key }
    }
}

struct PgLeaderGuard {
    conn: PgConn,
    key: i64,
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
                conn,
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
        advisory_lock::connection_alive(&mut self.conn).await
    }

    async fn release(mut self: Box<Self>) {
        let _ = advisory_lock::advisory_unlock(&mut self.conn, self.key).await;
    }
}
