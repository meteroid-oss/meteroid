//! Token-bucket rate limiting over a pluggable backend shared by every replica. Without a
//! backend every request is allowed.

mod config;

pub mod middleware;

use std::sync::Arc;
use std::time::UNIX_EPOCH;

pub use config::{RateLimitConfig, RateLimitResult};

pub type RateLimitBackendError = Box<dyn std::error::Error + Send + Sync>;

/// Where the buckets live.
#[async_trait::async_trait]
pub trait RateLimitBackend: Send + Sync {
    /// Takes `tokens` from the bucket at `key` atomically; negative `tokens` give them back.
    async fn try_check_n(
        &self,
        key: &str,
        config: &RateLimitConfig,
        tokens: f64,
    ) -> Result<RateLimitResult, RateLimitBackendError>;
}

/// Clone is cheap: the backend is shared.
#[derive(Clone, Default)]
pub struct RateLimiter {
    backend: Option<Arc<dyn RateLimitBackend>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(backend: Arc<dyn RateLimitBackend>, config: RateLimitConfig) -> Self {
        Self {
            backend: Some(backend),
            config,
        }
    }

    /// Derive a new limiter that shares the same backend but enforces a different
    /// rate limit configuration.
    pub fn with_config(&self, config: RateLimitConfig) -> Self {
        Self {
            backend: self.backend.clone(),
            config,
        }
    }

    /// Check whether this request should be allowed, consuming 1 token.
    pub async fn check<K: ToString>(&self, key: K) -> RateLimitResult {
        self.check_n(key, 1.0).await
    }

    /// A backend error allows the request.
    pub async fn check_n<K: ToString>(&self, key: K, tokens: f64) -> RateLimitResult {
        let Some(backend) = &self.backend else {
            return self.allowed();
        };
        match backend
            .try_check_n(&self.full_key(key), &self.config, tokens)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("Rate limit backend error: {e}. Allowing request.");
                self.allowed()
            }
        }
    }

    /// `check` for a caller that must not fail open: None when no answer could be had (no
    /// backend, or a backend error), so it can fall back to its own count.
    pub async fn try_check<K: ToString>(&self, key: K) -> Option<RateLimitResult> {
        let backend = self.backend.as_ref()?;
        match backend
            .try_check_n(&self.full_key(key), &self.config, 1.0)
            .await
        {
            Ok(result) => Some(result),
            Err(e) => {
                tracing::error!("Rate limit backend error: {e}. Falling back.");
                None
            }
        }
    }

    fn full_key<K: ToString>(&self, key: K) -> String {
        match &self.config.bucket {
            Some(bucket) => format!("{}:{}", bucket, key.to_string()),
            None => key.to_string(),
        }
    }

    fn allowed(&self) -> RateLimitResult {
        let limit = self.config.capacity as u64;
        RateLimitResult::Allowed {
            limit,
            remaining: limit,
            reset: UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() + self.config.window_seconds,
        }
    }

    /// Gives back one token taken by `check`; the bucket's next refill caps it at its capacity.
    pub async fn refund<K: ToString>(&self, key: K) {
        self.check_n(key, -1.0).await;
    }

    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    /// Returns `true` if a backend is installed.
    pub fn is_enabled(&self) -> bool {
        self.backend.is_some()
    }
}
