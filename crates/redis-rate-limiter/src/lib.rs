//! The Redis backend of the rate limiter: one token bucket per key, updated atomically by a Lua
//! script, so every replica shares the same limits.

use fred::prelude::*;
use fred::types::{ClusterHash, CustomCommand};
use rate_limiter::{RateLimitBackend, RateLimitBackendError, RateLimitConfig, RateLimitResult};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// Clone is cheap: fred's `Client` is a handle to the connection pool, the SHA is shared.
#[derive(Clone)]
pub struct RedisRateLimitBackend {
    client: Client,
    script_sha: Option<Arc<str>>,
}

impl RedisRateLimitBackend {
    /// Build the backend on top of an existing `fred::Client`.
    pub async fn new(client: Client) -> Self {
        let script = Self::token_bucket_script();
        let script_sha = match Self::load_script(&client, script).await {
            Ok(sha) => {
                tracing::info!("Loaded rate limit Lua script with SHA: {sha}");
                Some(Arc::from(sha))
            }
            Err(e) => {
                tracing::warn!("Failed to load Lua script: {e}. Will use fallback method.");
                None
            }
        };
        tracing::info!("Rate limiter enabled with Redis backend");
        Self { client, script_sha }
    }

    /// Token bucket implementation in Lua for atomic operations
    fn token_bucket_script() -> &'static str {
        r#"
        local key = KEYS[1]
        local capacity = tonumber(ARGV[1])
        local refill_rate = tonumber(ARGV[2])
        local now = tonumber(ARGV[3])
        local requested = tonumber(ARGV[4])
        local ttl = tonumber(ARGV[5])

        -- Get current state or initialize with full capacity
        local state = redis.call('HMGET', key, 'tokens', 'last_refill')
        local tokens = tonumber(state[1])
        local last_refill = tonumber(state[2])

        if tokens == nil then
            tokens = capacity
            last_refill = now
        end

        -- Refill tokens based on elapsed time
        local time_elapsed = now - last_refill
        if time_elapsed > 0 then
            local tokens_to_add = time_elapsed * refill_rate
            tokens = math.min(capacity, tokens + tokens_to_add)
            last_refill = now
        end

        -- Try to consume tokens
        if tokens >= requested then
            tokens = tokens - requested
            redis.call('HMSET', key, 'tokens', tostring(tokens), 'last_refill', tostring(now))
            redis.call('EXPIRE', key, ttl)
            return {1, tokens}
        else
            -- Not enough tokens, calculate retry time
            local tokens_needed = requested - tokens
            local retry_after = math.ceil(tokens_needed / refill_rate)
            -- Update state even on failure to track last access time
            redis.call('HMSET', key, 'tokens', tostring(tokens), 'last_refill', tostring(now))
            redis.call('EXPIRE', key, ttl)
            return {0, tokens, retry_after}
        end
        "#
    }

    /// Load Lua script into Redis and return its SHA
    async fn load_script(client: &Client, script: &str) -> Result<String, Error> {
        let cmd = CustomCommand::new("SCRIPT", ClusterHash::Random, false);
        let args = vec!["LOAD", script];
        let result: String = client.custom(cmd, args).await?;
        Ok(result)
    }

    /// Key format: "rate_limit:{key}"
    async fn check(
        &self,
        key: &str,
        config: &RateLimitConfig,
        tokens: f64,
    ) -> Result<RateLimitResult, Error> {
        let key = format!("rate_limit:{key}");
        let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs_f64();

        let ttl = config.window_seconds * 2;

        if let Some(ref sha) = self.script_sha {
            match self
                .run_script_sha(sha, &key, config, now, tokens, ttl)
                .await
            {
                // Redis dropped the cached script (restart, failover, SCRIPT FLUSH, or a
                // cluster node that never saw it). EVAL re-runs and re-caches it under the
                // same SHA, so later EVALSHA calls recover without a process restart.
                Err(e) if Self::is_noscript(&e) => {
                    self.run_script_eval(&key, config, now, tokens, ttl).await
                }
                other => other,
            }
        } else {
            self.run_script_eval(&key, config, now, tokens, ttl).await
        }
    }

    /// True when Redis rejected an EVALSHA because the script is no longer cached.
    fn is_noscript(e: &Error) -> bool {
        e.details().starts_with("NOSCRIPT")
    }

    async fn run_script_sha(
        &self,
        sha: &str,
        key: &str,
        config: &RateLimitConfig,
        now: f64,
        tokens_per_request: f64,
        ttl: u64,
    ) -> Result<RateLimitResult, Error> {
        let limit = config.capacity as u64;
        let reset = (now as u64) + config.window_seconds;

        let cmd = CustomCommand::new("EVALSHA", ClusterHash::FirstKey, false);
        let args = vec![
            sha.to_string(),
            "1".to_string(),
            key.to_string(),
            config.capacity.to_string(),
            config.refill_rate.to_string(),
            now.to_string(),
            tokens_per_request.to_string(),
            ttl.to_string(),
        ];

        let result: Vec<i64> = self.client.custom(cmd, args).await?;
        Ok(Self::parse_result(result, limit, reset))
    }

    async fn run_script_eval(
        &self,
        key: &str,
        config: &RateLimitConfig,
        now: f64,
        tokens_per_request: f64,
        ttl: u64,
    ) -> Result<RateLimitResult, Error> {
        let limit = config.capacity as u64;
        let reset = (now as u64) + config.window_seconds;

        let script = Self::token_bucket_script();
        let cmd = CustomCommand::new("EVAL", ClusterHash::FirstKey, false);
        let args = vec![
            script.to_string(),
            "1".to_string(),
            key.to_string(),
            config.capacity.to_string(),
            config.refill_rate.to_string(),
            now.to_string(),
            tokens_per_request.to_string(),
            ttl.to_string(),
        ];

        let result: Vec<i64> = self.client.custom(cmd, args).await?;
        Ok(Self::parse_result(result, limit, reset))
    }

    fn parse_result(result: Vec<i64>, limit: u64, reset: u64) -> RateLimitResult {
        if result.is_empty() {
            return RateLimitResult::Allowed {
                limit,
                remaining: limit,
                reset,
            };
        }

        if result[0] == 1 {
            let remaining = result.get(1).copied().unwrap_or(0).max(0) as u64;
            RateLimitResult::Allowed {
                limit,
                remaining,
                reset,
            }
        } else {
            let retry_after = result.get(2).copied().unwrap_or(1) as u64;
            RateLimitResult::RateLimited {
                limit,
                reset,
                retry_after,
            }
        }
    }
}

#[async_trait::async_trait]
impl RateLimitBackend for RedisRateLimitBackend {
    async fn try_check_n(
        &self,
        key: &str,
        config: &RateLimitConfig,
        tokens: f64,
    ) -> Result<RateLimitResult, RateLimitBackendError> {
        self.check(key, config, tokens).await.map_err(Into::into)
    }
}
