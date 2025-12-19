use crate::api_rest::webhooks::out_model::WebhookOutEvent;
use crate::config::SvixConfig;
use common_domain::ids::TenantId;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Jitter, Quota, RateLimiter, clock};
use nonzero::nonzero;
use secrecy::ExposeSecret;
use std::sync::Arc;
use std::time::Duration;
use svix::api::{
    AppPortalAccessIn, AppPortalAccessOut, ApplicationIn, EventTypeImportOpenApiIn,
    EventTypeImportOpenApiOut, MessageOut, Svix,
};
use svix::error::Error;

static API_RATE_LIMITER: std::sync::OnceLock<
    RateLimiter<NotKeyed, InMemoryState, clock::DefaultClock, NoOpMiddleware>,
> = std::sync::OnceLock::new();

struct ApiRateLimiter;

impl ApiRateLimiter {
    pub fn get()
    -> &'static RateLimiter<NotKeyed, InMemoryState, clock::DefaultClock, NoOpMiddleware> {
        API_RATE_LIMITER.get_or_init(|| RateLimiter::direct(Quota::per_second(nonzero!(50u32))))
    }
}

pub fn new_svix(config: &SvixConfig) -> Option<Arc<Svix>> {
    config.server_url.as_ref().map(|x| {
        Arc::new(Svix::new(
            config.token.expose_secret().to_string(),
            Some(svix::api::SvixOptions {
                debug: true,
                server_url: Some(x.clone()),
                timeout: Some(Duration::from_secs(30)),
                num_retries: Some(3),
                retry_schedule: None,
                proxy_address: None,
            }),
        ))
    })
}

#[async_trait::async_trait]
pub trait SvixOps {
    async fn create_message(
        &self,
        tenant_id: TenantId,
        msg: WebhookOutEvent,
    ) -> Result<MessageOut, Error>;

    async fn app_portal_access(&self, tenant_id: TenantId) -> Result<AppPortalAccessOut, Error>;

    async fn import_open_api_event_types(
        &self,
        schema: &str,
    ) -> Result<EventTypeImportOpenApiOut, Error>;
}

#[async_trait::async_trait]
impl SvixOps for Arc<Svix> {
    async fn create_message(
        &self,
        tenant_id: TenantId,
        msg: WebhookOutEvent,
    ) -> Result<MessageOut, Error> {
        let msg_in = msg
            .try_into()
            .map_err(|e: serde_json::Error| Error::Generic(e.to_string()))?;
        ApiRateLimiter::get()
            .until_ready_with_jitter(Jitter::up_to(Duration::from_secs(1)))
            .await;
        self.message()
            .create(tenant_id.to_string(), msg_in, None)
            .await
    }

    async fn app_portal_access(&self, tenant_id: TenantId) -> Result<AppPortalAccessOut, Error> {
        let app_in = ApplicationIn {
            metadata: None,
            name: tenant_id.to_string(),
            rate_limit: None,
            uid: Some(tenant_id.to_string()),
        };

        let access_in = AppPortalAccessIn {
            application: Some(app_in),
            capabilities: None,
            expiry: None, // 7 days by default
            feature_flags: None,
            #[allow(deprecated)]
            read_only: None,
            session_id: None,
        };

        ApiRateLimiter::get()
            .until_ready_with_jitter(Jitter::up_to(Duration::from_secs(1)))
            .await;

        self.authentication()
            .app_portal_access(tenant_id.to_string(), access_in, None)
            .await
    }

    async fn import_open_api_event_types(
        &self,
        schema: &str,
    ) -> Result<EventTypeImportOpenApiOut, Error> {
        ApiRateLimiter::get()
            .until_ready_with_jitter(Jitter::up_to(Duration::from_secs(1)))
            .await;
        self.event_type()
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
}
