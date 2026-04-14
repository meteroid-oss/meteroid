use crate::config::SvixConfig;
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
    EventTypeImportOpenApiOut, MessageIn, MessageOut, Svix,
};
use svix::error::Error;

/// Fallback in-memory rate limiter, used when Redis is unavailable.
static FALLBACK_RATE_LIMITER: std::sync::OnceLock<
    RateLimiter<NotKeyed, InMemoryState, clock::DefaultClock, NoOpMiddleware>,
> = std::sync::OnceLock::new();

fn fallback_limiter(
    rps: u32,
) -> &'static RateLimiter<NotKeyed, InMemoryState, clock::DefaultClock, NoOpMiddleware> {
    FALLBACK_RATE_LIMITER.get_or_init(|| {
        let rps = NonZero::new(rps).unwrap_or(nonzero!(25u32));
        RateLimiter::direct(Quota::per_second(rps))
    })
}

pub fn new_svix(config: &SvixConfig) -> Option<Arc<Svix>> {
    config
        .server_url
        .as_ref()
        .zip(config.token.as_ref())
        .map(|(url, token)| {
            log::info!("Initializing Svix client with server URL: {}", url);

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

// ── SvixOps trait ──────────────────────────────────────────────

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
}

// ── Redis-based rate limiter (shared across instances) ─────────

const RATE_LIMIT_WINDOW_PREFIX: &str = "svix:rate_limit:window";
const RATE_LIMIT_BACKOFF_KEY: &str = "svix:rate_limit:backoff";
const DEFAULT_429_BACKOFF_SECS: i64 = 5;

pub struct SvixRateLimiter {
    redis: Option<fred::prelude::Client>,
    rps_quota: u32,
}

impl SvixRateLimiter {
    pub fn new(redis: Option<fred::prelude::Client>, rps_quota: u32) -> Self {
        Self { redis, rps_quota }
    }

    pub async fn wait_for_permit(&self) {
        match &self.redis {
            Some(client) => self.redis_wait(client).await,
            None => {
                fallback_limiter(self.rps_quota)
                    .until_ready_with_jitter(Jitter::up_to(Duration::from_secs(1)))
                    .await;
            }
        }
    }

    async fn redis_wait(&self, client: &fred::prelude::Client) {
        // Check global backoff (set on 429)
        if let Ok(Some(backoff_until)) = client
            .get::<Option<i64>, _>(RATE_LIMIT_BACKOFF_KEY)
            .await
        {
            let now = chrono::Utc::now().timestamp();
            if backoff_until > now {
                let wait = Duration::from_secs((backoff_until - now) as u64);
                tracing::info!("Svix rate limit backoff: waiting {wait:?}");
                tokio::time::sleep(wait).await;
            }
        }

        // Fixed-window counter keyed to current second
        let now_sec = chrono::Utc::now().timestamp();
        let key = format!("{RATE_LIMIT_WINDOW_PREFIX}:{now_sec}");

        loop {
            let count: Result<i64, _> = client.incr(&key).await;
            match count {
                Ok(1) => {
                    // First increment — set expiry
                    let _: Result<(), _> =
                        client.expire(&key, 2, None).await;
                }
                Ok(n) if n > self.rps_quota as i64 => {
                    // Over quota — wait for next second window
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let next_sec_ms = (now_sec + 1) * 1000;
                    let wait_ms = (next_sec_ms - now_ms).max(10) as u64;
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("Redis rate limiter error: {e}. Falling back to governor.");
                    fallback_limiter(self.rps_quota)
                        .until_ready_with_jitter(Jitter::up_to(Duration::from_secs(1)))
                        .await;
                }
                _ => {} // within quota
            }
            break;
        }
    }

    /// Record a 429 backoff in Redis so all instances respect it.
    pub async fn record_429_backoff(&self) {
        if let Some(client) = &self.redis {
            let backoff_until = chrono::Utc::now().timestamp() + DEFAULT_429_BACKOFF_SECS;
            let result: Result<Option<String>, _> = client
                .set(
                    RATE_LIMIT_BACKOFF_KEY,
                    backoff_until,
                    Some(Expiration::EX(DEFAULT_429_BACKOFF_SECS)),
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

// ── SvixClient: wraps Svix + rate limiter, implements SvixOps ──

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

        if let Err(Error::Http(e)) = &result {
            if e.status.as_u16() == 429 {
                self.rate_limiter.record_429_backoff().await;
            }
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

            let mut opts = svix::api::EndpointListOptions::default();
            opts.iterator = iterator;

            let page = self
                .svix
                .endpoint()
                .list(app_id.clone(), Some(opts))
                .await?;
            all_endpoints.extend(page.data);

            if page.done {
                break;
            }
            iterator = page.iterator;
        }

        Ok(all_endpoints)
    }
}
