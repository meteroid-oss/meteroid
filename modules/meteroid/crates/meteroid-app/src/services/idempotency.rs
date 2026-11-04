use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fred::prelude::{Expiration, KeysInterface, SetOptions};
use tokio::sync::RwLock;

#[async_trait]
pub trait IdempotencyService: Send + Sync {
    /// Returns `true` if the key was already seen (duplicate request),
    /// or `false` if the key was new and has been recorded.
    async fn check_and_set(&self, key: String, ttl: Duration) -> bool;

    async fn invalidate(&self, key: String);
}

#[derive(Clone, Default)]
pub struct InMemoryIdempotencyService {
    seen: Arc<RwLock<HashSet<String>>>,
}

impl InMemoryIdempotencyService {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl IdempotencyService for InMemoryIdempotencyService {
    async fn check_and_set(&self, key: String, _ttl: Duration) -> bool {
        {
            let read = self.seen.read().await;
            if read.contains(&key) {
                return true;
            }
        }
        self.seen.write().await.insert(key);
        false
    }

    async fn invalidate(&self, key: String) {
        self.seen.write().await.remove(&key);
    }
}

/// Redis-backed idempotency service.
///
/// Clone is cheap — `fred::Client` is just a handle to the connection pool.
#[derive(Clone)]
pub struct RedisIdempotencyService {
    client: fred::prelude::Client,
}

impl RedisIdempotencyService {
    pub fn new(client: fred::prelude::Client) -> Self {
        Self { client }
    }

    fn redis_key(key: &str) -> String {
        format!("idempotency:{key}")
    }
}

#[async_trait]
impl IdempotencyService for RedisIdempotencyService {
    async fn check_and_set(&self, key: String, ttl: Duration) -> bool {
        let redis_key = Self::redis_key(&key);
        let ttl_ms = ttl.as_millis() as i64;

        // SET key "1" NX PX <ttl_ms>
        //   → Some("OK")  key was newly set  → first time   → not a duplicate
        //   → None        key already existed → duplicate
        let result: Result<Option<String>, _> = self
            .client
            .set(
                &redis_key,
                "1",
                Some(Expiration::PX(ttl_ms)),
                Some(SetOptions::NX),
                false,
            )
            .await;

        match result {
            Ok(Some(_)) => false, // key was set  → first time
            Ok(None) => true,     // key existed  → duplicate
            Err(e) => {
                tracing::error!(
                    "Redis error in idempotency check_and_set for key {redis_key}: {e}. \
                     Treating as new request."
                );
                false // fail open
            }
        }
    }

    async fn invalidate(&self, key: String) {
        let redis_key = Self::redis_key(&key);
        let result: Result<u64, _> = self.client.del(&redis_key).await;
        if let Err(e) = result {
            tracing::warn!("Redis error in idempotency invalidate for key {redis_key}: {e}");
        }
    }
}
