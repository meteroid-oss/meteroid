use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

#[async_trait]
pub trait IdempotencyService: Send + Sync {
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
