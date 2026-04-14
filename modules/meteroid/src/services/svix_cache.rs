use async_trait::async_trait;
use common_domain::ids::TenantId;
use fred::prelude::{Expiration, KeysInterface};
use serde::{Deserialize, Serialize};

const REDIS_KEY_PREFIX: &str = "svix:endpoints";
const CACHE_TTL_WITH_INVALIDATION: i64 = 7 * 24 * 3600; // 7 days (operational webhooks handle invalidation)
const CACHE_TTL_WITHOUT_INVALIDATION: i64 = 5 * 60; // 5 minutes (no operational webhooks, must re-check periodically)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    /// At least one endpoint has no filter_types (listens to all event types).
    pub wildcard: bool,
    /// Union of filter_types across all non-disabled endpoints.
    pub event_types: Vec<String>,
    /// Tenant has zero active endpoints.
    pub empty: bool,
}

impl EndpointConfig {
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

#[async_trait]
pub trait SvixEndpointCache: Send + Sync {
    /// Returns Some(true/false) if cached, None on cache miss.
    async fn should_send(&self, tenant_id: &TenantId, event_type: &str) -> Option<bool>;

    /// Store the endpoint configuration for a tenant.
    async fn store(&self, tenant_id: &TenantId, config: &EndpointConfig);

    /// Invalidate (delete) the cache for a tenant.
    async fn invalidate(&self, tenant_id: &TenantId);
}

fn redis_key(tenant_id: &TenantId) -> String {
    format!("{REDIS_KEY_PREFIX}:{tenant_id}")
}

/// Redis-backed endpoint cache. Fail-open on all Redis errors.
pub struct RedisSvixEndpointCache {
    client: fred::prelude::Client,
    ttl_seconds: i64,
}

impl RedisSvixEndpointCache {
    pub fn new(client: fred::prelude::Client, has_operational_webhooks: bool) -> Self {
        let ttl_seconds = if has_operational_webhooks {
            CACHE_TTL_WITH_INVALIDATION
        } else {
            CACHE_TTL_WITHOUT_INVALIDATION
        };
        Self { client, ttl_seconds }
    }
}

#[async_trait]
impl SvixEndpointCache for RedisSvixEndpointCache {
    async fn should_send(&self, tenant_id: &TenantId, event_type: &str) -> Option<bool> {
        let key = redis_key(tenant_id);
        let result: Result<Option<String>, _> = self.client.get(&key).await;
        match result {
            Ok(Some(json)) => match serde_json::from_str::<EndpointConfig>(&json) {
                Ok(config) => Some(config.should_send(event_type)),
                Err(e) => {
                    tracing::warn!("Failed to deserialize endpoint cache for {key}: {e}");
                    None
                }
            },
            Ok(None) => None, // cache miss
            Err(e) => {
                tracing::warn!("Redis error reading endpoint cache for {key}: {e}");
                None // fail-open
            }
        }
    }

    async fn store(&self, tenant_id: &TenantId, config: &EndpointConfig) {
        let key = redis_key(tenant_id);
        match serde_json::to_string(config) {
            Ok(json) => {
                let result: Result<Option<String>, _> = self
                    .client
                    .set(&key, json, Some(Expiration::EX(self.ttl_seconds)), None, false)
                    .await;
                if let Err(e) = result {
                    tracing::warn!("Redis error storing endpoint cache for {key}: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("Failed to serialize endpoint cache for {key}: {e}");
            }
        }
    }

    async fn invalidate(&self, tenant_id: &TenantId) {
        let key = redis_key(tenant_id);
        let result: Result<u64, _> = self.client.del(&key).await;
        if let Err(e) = result {
            tracing::warn!("Redis error invalidating endpoint cache for {key}: {e}");
        }
    }
}

/// No-op cache that always sends. Used when Redis is not configured.
pub struct NoopSvixEndpointCache;

#[async_trait]
impl SvixEndpointCache for NoopSvixEndpointCache {
    async fn should_send(&self, _tenant_id: &TenantId, _event_type: &str) -> Option<bool> {
        Some(true)
    }

    async fn store(&self, _tenant_id: &TenantId, _config: &EndpointConfig) {}

    async fn invalidate(&self, _tenant_id: &TenantId) {}
}

/// Build an EndpointConfig from a list of Svix endpoints.
pub fn build_endpoint_config(endpoints: &[svix::api::EndpointOut]) -> EndpointConfig {
    let active: Vec<_> = endpoints
        .iter()
        .filter(|ep| ep.disabled != Some(true))
        .collect();

    if active.is_empty() {
        return EndpointConfig {
            wildcard: false,
            event_types: vec![],
            empty: true,
        };
    }

    let mut wildcard = false;
    let mut event_types = std::collections::HashSet::new();

    for ep in &active {
        match &ep.filter_types {
            None => {
                wildcard = true;
            }
            Some(types) if types.is_empty() => {
                wildcard = true;
            }
            Some(types) => {
                event_types.extend(types.iter().cloned());
            }
        }
    }

    EndpointConfig {
        wildcard,
        event_types: event_types.into_iter().collect(),
        empty: false,
    }
}

