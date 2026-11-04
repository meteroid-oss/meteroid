#[derive(Clone, Debug, Default)]
pub struct RateLimitConfig {
    /// Maximum number of tokens (requests) in the bucket
    pub capacity: f64,
    /// Rate at which tokens are refilled (tokens per second)
    pub refill_rate: f64,
    /// Window duration in seconds (the backend expires idle buckets after two)
    pub window_seconds: u64,
    /// Optional bucket name prepended to the key: `{bucket}:{caller_key}`.
    pub bucket: Option<String>,
}

impl RateLimitConfig {
    pub fn new(capacity: f64, refill_rate: f64, window_seconds: u64) -> Self {
        Self {
            capacity,
            refill_rate,
            window_seconds,
            bucket: None,
        }
    }

    /// Attach a bucket namespace to this config (builder-style).
    pub fn with_bucket(mut self, bucket: impl Into<String>) -> Self {
        self.bucket = Some(bucket.into());
        self
    }

    /// Create a config for requests per minute
    ///
    /// Example: `RateLimitConfig::per_minute(60)` allows 60 requests per minute,
    /// with tokens refilling at 1 token per second.
    pub fn per_minute(requests: u64) -> Self {
        Self {
            capacity: requests as f64,
            refill_rate: requests as f64 / 60.0,
            window_seconds: 60,
            bucket: None,
        }
    }

    /// Create a config for requests per second
    ///
    /// Example: `RateLimitConfig::per_second(10)` allows 10 requests per second,
    /// with tokens refilling at 10 tokens per second.
    pub fn per_second(requests: u64) -> Self {
        Self {
            capacity: requests as f64,
            refill_rate: requests as f64,
            window_seconds: 1,
            bucket: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum RateLimitResult {
    /// Request is allowed
    Allowed {
        /// Maximum number of requests allowed
        limit: u64,
        /// Number of requests remaining
        remaining: u64,
        /// Unix timestamp when the rate limit resets
        reset: u64,
    },
    /// Request is rate limited
    RateLimited {
        /// Maximum number of requests allowed
        limit: u64,
        /// Unix timestamp when the rate limit resets
        reset: u64,
        /// Seconds to wait before retrying
        retry_after: u64,
    },
}

impl RateLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, RateLimitResult::Allowed { .. })
    }

    /// Get the rate limit (maximum requests allowed)
    pub fn limit(&self) -> u64 {
        match self {
            RateLimitResult::Allowed { limit, .. } => *limit,
            RateLimitResult::RateLimited { limit, .. } => *limit,
        }
    }

    /// Get remaining requests (None if rate limited)
    pub fn remaining(&self) -> Option<u64> {
        match self {
            RateLimitResult::Allowed { remaining, .. } => Some(*remaining),
            RateLimitResult::RateLimited { .. } => None,
        }
    }

    /// Get reset timestamp (Unix seconds)
    pub fn reset(&self) -> u64 {
        match self {
            RateLimitResult::Allowed { reset, .. } => *reset,
            RateLimitResult::RateLimited { reset, .. } => *reset,
        }
    }

    /// Get retry_after duration if rate limited
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            RateLimitResult::RateLimited { retry_after, .. } => Some(*retry_after),
            _ => None,
        }
    }
}
