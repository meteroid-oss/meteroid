use async_trait::async_trait;
use common_domain::ids::TenantId;
use fred::prelude::{Expiration, KeysInterface, SetOptions};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const REDIS_KEY_PREFIX: &str = "svix:endpoints";
const REDIS_LOCK_PREFIX: &str = "svix:endpoints:lock";
const REDIS_PORTAL_ACTIVE_PREFIX: &str = "svix:portal_active";

const CACHE_TTL_LONG_SECS: i64 = 7 * 24 * 3600;
const CACHE_TTL_SHORT_SECS: i64 = 5 * 60;
const CACHE_TTL_NO_OP_WEBHOOKS_SECS: i64 = 5 * 60;
const PORTAL_ACTIVE_TTL_SECS: i64 = 4 * 3600;

// Deduplication hint, not a mutex. Followers fall through to solo-load after LOCK_POLL_MAX
// regardless, so the TTL only bounds how long a stale key lingers after a dead holder.
const LOCK_TTL_SECS: i64 = 60;
const LOCK_POLL_MAX: Duration = Duration::from_millis(2000);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub wildcard: bool,
    pub event_types: Vec<String>,
    pub empty: bool,
}

impl EndpointConfig {
    pub fn empty() -> Self {
        Self {
            wildcard: false,
            event_types: vec![],
            empty: true,
        }
    }

    pub fn send_all() -> Self {
        Self {
            wildcard: true,
            event_types: vec![],
            empty: false,
        }
    }

    pub fn should_send(&self, event_type: &str) -> bool {
        if self.empty {
            return false;
        }
        if self.wildcard {
            return true;
        }
        self.event_types.iter().any(|et| et == event_type)
    }
}

pub enum LockOutcome {
    /// Caller holds the lock and must call `release_lock` after populating the cache.
    Acquired,
    CachedByOther(EndpointConfig),
    /// Lock taken by another caller, but their value didn't land in cache within the poll window
    /// — caller should load on its own.
    TimedOut,
    /// No Redis: act as solo caller.
    NoCache,
}

#[async_trait]
pub trait SvixEndpointCache: Send + Sync {
    async fn get(&self, tenant_id: &TenantId) -> Option<EndpointConfig>;
    async fn store(&self, tenant_id: &TenantId, config: &EndpointConfig);
    async fn invalidate(&self, tenant_id: &TenantId);
    async fn mark_portal_active(&self, tenant_id: &TenantId);
    async fn try_acquire_lock(&self, tenant_id: &TenantId) -> LockOutcome;
    async fn release_lock(&self, tenant_id: &TenantId);
}

fn cache_key(tenant_id: &TenantId) -> String {
    format!("{REDIS_KEY_PREFIX}:{tenant_id}")
}
fn lock_key(tenant_id: &TenantId) -> String {
    format!("{REDIS_LOCK_PREFIX}:{tenant_id}")
}
fn portal_key(tenant_id: &TenantId) -> String {
    format!("{REDIS_PORTAL_ACTIVE_PREFIX}:{tenant_id}")
}

pub struct RedisSvixEndpointCache {
    client: fred::prelude::Client,
    default_ttl_secs: i64,
}

impl RedisSvixEndpointCache {
    pub fn new(client: fred::prelude::Client, has_operational_webhooks: bool) -> Self {
        let default_ttl_secs = if has_operational_webhooks {
            CACHE_TTL_LONG_SECS
        } else {
            CACHE_TTL_NO_OP_WEBHOOKS_SECS
        };
        Self {
            client,
            default_ttl_secs,
        }
    }

    async fn is_portal_active(&self, tenant_id: &TenantId) -> bool {
        let result: Result<Option<String>, _> = self.client.get(&portal_key(tenant_id)).await;
        matches!(result, Ok(Some(_)))
    }
}

#[async_trait]
impl SvixEndpointCache for RedisSvixEndpointCache {
    async fn get(&self, tenant_id: &TenantId) -> Option<EndpointConfig> {
        let key = cache_key(tenant_id);
        let result: Result<Option<String>, _> = self.client.get(&key).await;
        match result {
            Ok(Some(json)) => match serde_json::from_str::<EndpointConfig>(&json) {
                Ok(config) => Some(config),
                Err(e) => {
                    tracing::warn!("Failed to deserialize endpoint cache for {key}: {e}");
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("Redis error reading endpoint cache for {key}: {e}");
                None
            }
        }
    }

    async fn store(&self, tenant_id: &TenantId, config: &EndpointConfig) {
        let ttl = if self.is_portal_active(tenant_id).await {
            CACHE_TTL_SHORT_SECS
        } else {
            self.default_ttl_secs
        };

        let key = cache_key(tenant_id);
        let json = match serde_json::to_string(config) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize endpoint cache for {key}: {e}");
                return;
            }
        };
        let result: Result<Option<String>, _> = self
            .client
            .set(&key, json, Some(Expiration::EX(ttl)), None, false)
            .await;
        if let Err(e) = result {
            tracing::warn!("Redis error storing endpoint cache for {key}: {e}");
        }
    }

    async fn invalidate(&self, tenant_id: &TenantId) {
        let key = cache_key(tenant_id);
        let result: Result<u64, _> = self.client.del(&key).await;
        if let Err(e) = result {
            tracing::warn!("Redis error invalidating endpoint cache for {key}: {e}");
        }
    }

    async fn mark_portal_active(&self, tenant_id: &TenantId) {
        let key = portal_key(tenant_id);
        let result: Result<Option<String>, _> = self
            .client
            .set(
                &key,
                "1",
                Some(Expiration::EX(PORTAL_ACTIVE_TTL_SECS)),
                None,
                false,
            )
            .await;
        if let Err(e) = result {
            tracing::warn!("Redis error marking portal active for {key}: {e}");
        }
    }

    async fn try_acquire_lock(&self, tenant_id: &TenantId) -> LockOutcome {
        let lkey = lock_key(tenant_id);
        let ckey = cache_key(tenant_id);

        let acquired: Result<Option<String>, _> = self
            .client
            .set(
                &lkey,
                "1",
                Some(Expiration::EX(LOCK_TTL_SECS)),
                Some(SetOptions::NX),
                false,
            )
            .await;

        match acquired {
            Ok(Some(_)) => LockOutcome::Acquired,
            Ok(None) => {
                let deadline = std::time::Instant::now() + LOCK_POLL_MAX;
                loop {
                    tokio::time::sleep(LOCK_POLL_INTERVAL).await;
                    let got: Result<Option<String>, _> = self.client.get(&ckey).await;
                    if let Ok(Some(json)) = got
                        && let Ok(config) = serde_json::from_str::<EndpointConfig>(&json)
                    {
                        return LockOutcome::CachedByOther(config);
                    }
                    if std::time::Instant::now() >= deadline {
                        return LockOutcome::TimedOut;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Redis error acquiring single-flight lock for {lkey}: {e}");
                LockOutcome::TimedOut
            }
        }
    }

    async fn release_lock(&self, tenant_id: &TenantId) {
        let key = lock_key(tenant_id);
        let result: Result<u64, _> = self.client.del(&key).await;
        if let Err(e) = result {
            tracing::warn!("Redis error releasing single-flight lock for {key}: {e}");
        }
    }
}

/// Used when Redis is absent: always send, never cache
pub struct NoopSvixEndpointCache;

#[async_trait]
impl SvixEndpointCache for NoopSvixEndpointCache {
    async fn get(&self, _tenant_id: &TenantId) -> Option<EndpointConfig> {
        Some(EndpointConfig::send_all())
    }
    async fn store(&self, _tenant_id: &TenantId, _config: &EndpointConfig) {}
    async fn invalidate(&self, _tenant_id: &TenantId) {}
    async fn mark_portal_active(&self, _tenant_id: &TenantId) {}
    async fn try_acquire_lock(&self, _tenant_id: &TenantId) -> LockOutcome {
        LockOutcome::NoCache
    }
    async fn release_lock(&self, _tenant_id: &TenantId) {}
}

pub fn build_endpoint_config(endpoints: &[svix::api::EndpointOut]) -> EndpointConfig {
    let mut wildcard = false;
    let mut event_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut has_active = false;

    for ep in endpoints {
        if ep.disabled == Some(true) {
            continue;
        }
        has_active = true;
        match &ep.filter_types {
            None => wildcard = true,
            Some(types) if types.is_empty() => wildcard = true,
            Some(types) => event_types.extend(types.iter().cloned()),
        }
    }

    if !has_active {
        return EndpointConfig::empty();
    }

    EndpointConfig {
        wildcard,
        event_types: event_types.into_iter().collect(),
        empty: false,
    }
}
