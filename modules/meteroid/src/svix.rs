use crate::api_rest::svix_operational::SvixOperationalState;
use crate::config::SvixConfig;
use crate::services::svix_cache::{
    NoopSvixEndpointCache, RedisSvixEndpointCache, SvixEndpointCache,
};
use common_domain::ids::TenantId;
use fred::prelude::{Expiration, KeysInterface};
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter, clock};
use nonzero::nonzero;
use secrecy::ExposeSecret;
use std::num::NonZero;
use std::sync::Arc;
use std::time::Duration;
use svix::api::{
    AppPortalAccessIn, AppPortalAccessOut, ApplicationIn, EndpointOut, EventTypeImportOpenApiIn,
    EventTypeImportOpenApiOut, MessageIn, MessageOut, OperationalWebhookEndpointListOptions,
    OperationalWebhookEndpointOut, Svix,
};
use svix::error::Error;

type GovernorLimiter = RateLimiter<NotKeyed, InMemoryState, clock::DefaultClock, NoOpMiddleware>;

fn build_limiter(rps: u32) -> GovernorLimiter {
    let rps = NonZero::new(rps).unwrap_or(nonzero!(25u32));
    RateLimiter::direct(Quota::per_second(rps))
}

pub fn new_svix(config: &SvixConfig) -> Option<Arc<Svix>> {
    config
        .server_url
        .as_ref()
        .zip(config.token.as_ref())
        .map(|(url, token)| {
            tracing::info!("Initializing Svix client with server URL: {}", url);

            Arc::new(Svix::new(
                token.expose_secret().to_string(),
                Some(svix::api::SvixOptions {
                    debug: true,
                    server_url: Some(url.clone()),
                    timeout: Some(Duration::from_secs(30)),
                    num_retries: Some(3),
                    retry_schedule: None,
                    proxy_address: None,
                }),
            ))
        })
}

#[async_trait::async_trait]
pub trait SvixOps: Send + Sync {
    async fn create_message(
        &self,
        tenant_id: TenantId,
        msg: MessageIn,
    ) -> Result<MessageOut, Error>;

    async fn app_portal_access(&self, tenant_id: TenantId) -> Result<AppPortalAccessOut, Error>;

    async fn import_open_api_event_types(
        &self,
        schema: &str,
    ) -> Result<EventTypeImportOpenApiOut, Error>;

    async fn list_endpoints(&self, tenant_id: TenantId) -> Result<Vec<EndpointOut>, Error>;

    async fn list_operational_webhook_endpoints(
        &self,
    ) -> Result<Vec<OperationalWebhookEndpointOut>, Error>;
}

const RATE_LIMIT_WINDOW_PREFIX: &str = "svix:rate_limit:window";
const RATE_LIMIT_BACKOFF_KEY: &str = "svix:rate_limit:backoff";

pub struct SvixRateLimiter {
    redis: Option<fred::prelude::Client>,
    rps_quota: u32,
    backoff_429_secs: i64,
    fallback: GovernorLimiter,
}

impl SvixRateLimiter {
    pub fn new(
        redis: Option<fred::prelude::Client>,
        rps_quota: u32,
        backoff_429_secs: u32,
    ) -> Self {
        Self {
            redis,
            rps_quota,
            backoff_429_secs: backoff_429_secs as i64,
            fallback: build_limiter(rps_quota),
        }
    }

    pub async fn wait_for_permit(&self) {
        match &self.redis {
            Some(client) => self.redis_wait(client).await,
            None => self.fallback_wait().await,
        }
    }

    async fn fallback_wait(&self) {
        self.fallback
            .until_ready_with_jitter(Jitter::up_to(Duration::from_secs(1)))
            .await;
    }

    async fn redis_wait(&self, client: &fred::prelude::Client) {
        if let Ok(Some(backoff_until)) = client.get::<Option<i64>, _>(RATE_LIMIT_BACKOFF_KEY).await
        {
            let now = chrono::Utc::now().timestamp();
            if backoff_until > now {
                let wait = Duration::from_secs((backoff_until - now) as u64);
                tracing::info!("Svix rate limit backoff: waiting {wait:?}");
                tokio::time::sleep(wait).await;
            }
        }

        loop {
            let now_sec = chrono::Utc::now().timestamp();
            let key = format!("{RATE_LIMIT_WINDOW_PREFIX}:{now_sec}");

            let count: Result<i64, _> = client.incr(&key).await;
            match count {
                Ok(1) => {
                    let _: Result<(), _> = client.expire(&key, 2, None).await;
                }
                Ok(n) if n > self.rps_quota as i64 => {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let next_sec_ms = (now_sec + 1) * 1000;
                    let wait_ms = (next_sec_ms - now_ms).max(10) as u64;
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("Redis rate limiter error: {e}. Falling back to governor.");
                    self.fallback_wait().await;
                }
                _ => {} // within quota
            }
            break;
        }
    }

    /// Shared across instances. Only overwrites if our target is later, so concurrent 429s can't shorten the window.
    pub async fn record_429_backoff(&self) {
        let Some(client) = &self.redis else { return };
        let backoff_until = chrono::Utc::now().timestamp() + self.backoff_429_secs;

        let current: Result<Option<i64>, _> = client.get(RATE_LIMIT_BACKOFF_KEY).await;
        let should_write = !matches!(current, Ok(Some(existing)) if existing >= backoff_until);

        if should_write {
            let result: Result<Option<String>, _> = client
                .set(
                    RATE_LIMIT_BACKOFF_KEY,
                    backoff_until,
                    Some(Expiration::EX(self.backoff_429_secs)),
                    None,
                    false,
                )
                .await;
            if let Err(e) = result {
                tracing::warn!("Failed to set Svix rate limit backoff in Redis: {e}");
            }
        }
    }
}

pub struct SvixClient {
    svix: Arc<Svix>,
    rate_limiter: Arc<SvixRateLimiter>,
}

impl SvixClient {
    pub fn new(svix: Arc<Svix>, rate_limiter: Arc<SvixRateLimiter>) -> Self {
        Self { svix, rate_limiter }
    }
}

#[async_trait::async_trait]
impl SvixOps for SvixClient {
    async fn create_message(
        &self,
        tenant_id: TenantId,
        msg: MessageIn,
    ) -> Result<MessageOut, Error> {
        self.rate_limiter.wait_for_permit().await;
        let result = self
            .svix
            .message()
            .create(tenant_id.to_string(), msg, None)
            .await;

        if let Err(Error::Http(e)) = &result
            && e.status.as_u16() == 429
        {
            self.rate_limiter.record_429_backoff().await;
        }

        result
    }

    async fn app_portal_access(&self, tenant_id: TenantId) -> Result<AppPortalAccessOut, Error> {
        let mut app_in = ApplicationIn::new(tenant_id.to_string());
        app_in.uid = Some(tenant_id.to_string());

        let access_in = AppPortalAccessIn {
            application: Some(app_in),
            capabilities: None,
            expiry: None, // 7 days by default
            feature_flags: None,
            #[allow(deprecated)]
            read_only: None,
            session_id: None,
        };

        self.rate_limiter.wait_for_permit().await;

        self.svix
            .authentication()
            .app_portal_access(tenant_id.to_string(), access_in, None)
            .await
    }

    async fn import_open_api_event_types(
        &self,
        schema: &str,
    ) -> Result<EventTypeImportOpenApiOut, Error> {
        self.rate_limiter.wait_for_permit().await;
        self.svix
            .event_type()
            .import_openapi(
                EventTypeImportOpenApiIn {
                    dry_run: None,
                    replace_all: Some(true),
                    spec: None,
                    spec_raw: Some(schema.to_string()),
                },
                None,
            )
            .await
    }

    async fn list_endpoints(&self, tenant_id: TenantId) -> Result<Vec<EndpointOut>, Error> {
        let app_id = tenant_id.to_string();
        let mut all_endpoints = Vec::new();
        let mut iterator: Option<String> = None;

        loop {
            self.rate_limiter.wait_for_permit().await;

            let opts = svix::api::EndpointListOptions {
                iterator: iterator.clone(),
                ..Default::default()
            };

            let page = self
                .svix
                .endpoint()
                .list(app_id.clone(), Some(opts))
                .await?;
            all_endpoints.extend(page.data);

            // Defensive: break on done, or if Svix returns a non-advancing iterator.
            if page.done || page.iterator.is_none() || page.iterator == iterator {
                break;
            }
            iterator = page.iterator;
        }

        Ok(all_endpoints)
    }

    async fn list_operational_webhook_endpoints(
        &self,
    ) -> Result<Vec<OperationalWebhookEndpointOut>, Error> {
        let mut all = Vec::new();
        let mut iterator: Option<String> = None;

        loop {
            self.rate_limiter.wait_for_permit().await;

            let opts = OperationalWebhookEndpointListOptions {
                iterator: iterator.clone(),
                ..Default::default()
            };

            let page = self
                .svix
                .operational_webhook()
                .endpoint()
                .list(Some(opts))
                .await?;
            all.extend(page.data);

            if page.done || page.iterator.is_none() || page.iterator == iterator {
                break;
            }
            iterator = page.iterator;
        }

        Ok(all)
    }
}

pub struct SvixWiring {
    pub svix: Option<Arc<dyn SvixOps>>,
    pub endpoint_cache: Arc<dyn SvixEndpointCache>,
    pub op_webhook_state: Option<SvixOperationalState>,
}

/// Wires the Svix client, rate limiter, endpoint cache, and operational-webhook
/// state from config + an optional shared Redis client.
pub fn wire_svix(config: &SvixConfig, fred_client: Option<fred::prelude::Client>) -> SvixWiring {
    let has_op_secret = config.operational_webhook_secret.is_some();

    let endpoint_cache: Arc<dyn SvixEndpointCache> = match &fred_client {
        Some(client) => Arc::new(RedisSvixEndpointCache::new(client.clone(), has_op_secret)),
        None => Arc::new(NoopSvixEndpointCache),
    };

    let rate_limiter = Arc::new(SvixRateLimiter::new(
        fred_client,
        config.rps_quota,
        config.rate_limit_429_backoff_secs,
    ));

    let svix: Option<Arc<dyn SvixOps>> =
        new_svix(config).map(|s| Arc::new(SvixClient::new(s, rate_limiter)) as Arc<dyn SvixOps>);

    let op_webhook_state = config.operational_webhook_secret.as_ref().map(|secret| {
        let verifier =
            svix::webhooks::Webhook::new(secret).expect("Invalid SVIX_OPERATIONAL_WEBHOOK_SECRET");
        SvixOperationalState::new(Arc::new(verifier), endpoint_cache.clone())
    });

    SvixWiring {
        svix,
        endpoint_cache,
        op_webhook_state,
    }
}
