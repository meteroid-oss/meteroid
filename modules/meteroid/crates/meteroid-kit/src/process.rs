use crate::Modules;
use common_logging::GLOBAL_METER;
use fred::prelude::{Builder, ClientLike, Config as FredConfig, ReconnectPolicy};
use meteroid_app::clients::usage::MeteringUsageClient;
use meteroid_app::config::{Config, RedisConfig, WebhookOutConfig};
use meteroid_app::eventbus::create_eventbus_memory;
use meteroid_app::services::egress::{Egress, EgressPurpose};
use meteroid_store::Store;
use meteroid_store::store::{PgPool, StoreConfig};
use opentelemetry::KeyValue;
use opentelemetry::metrics::ObservableGauge;
use rate_limiter::{RateLimitConfig, RateLimiter};
use redis_rate_limiter::RedisRateLimitBackend;
use secrecy::ExposeSecret;
use std::sync::Arc;
use std::time::Duration;

/// The process's store, built once by the binary and passed down. `usage_client` is the
/// process's metering client: the store holds it, and it reads tenants through the store.
pub fn build_store(
    modules: &Modules,
    config: &Config,
    usage_client: &MeteringUsageClient,
    egress: &Egress,
) -> Store {
    let mailer = meteroid_mailer::service::mailer_service(config.mailer.clone());
    let sso_egress = egress
        .client(EgressPurpose::Sso)
        .expect("Failed to build the SSO egress client");
    let oauth = meteroid_oauth::service::OauthServices::new(config.oauth.clone(), sso_egress);

    let store = Store::new(StoreConfig {
        pg: config.pg.clone(),
        crypt_key: config.secrets_crypt_key.0.clone(),
        jwt_secret: config.jwt_secret.clone(),
        multi_organization_enabled: config.multi_organization_enabled,
        mailer_enabled: config.mailer_enabled(),
        public_url: config.public_url.clone(),
        rest_api_external_url: config.rest_api_external_url.clone(),
        webhook_external_url: config.webhook_external_url.clone(),
        eventbus: create_eventbus_memory(),
        mailer,
        oauth,
        domains_whitelist: config.domains_whitelist(),
        organization_hooks: modules.edition.organization_hooks(config),
        instance: modules.edition.instance(),
        hooks: modules.store_hooks(),
        admin_organization_id: config.admin_organization_id,
        usage_client: Arc::new(usage_client.clone()),
        invite_ttl_days: config.invite_ttl_days,
        mfa_issuer: config.mfa_issuer.clone(),
    })
    .expect("Failed to initialize store");

    usage_client.bind_tenant_lookup(store.keep_last_events_lookup());
    register_pool_metrics(&store.pool);

    store
}

/// The process's outbound HTTP clients. Private and internal destinations are refused unless
/// `WEBHOOK_ALLOW_PRIVATE_DESTINATIONS` (or its SSO override) allows them.
pub fn egress(config: &WebhookOutConfig) -> Egress {
    let guard = egress_guard::DestinationGuard::new(
        config.allow_private_destinations,
        config.sso_allow_private_destinations,
    );
    Egress::new(config.clone(), Some(Arc::new(guard)))
}

/// The API's rate limiter, shared by every replica through Redis. Without Redis, requests are
/// not limited.
pub async fn rate_limiter(redis: Option<&fred::prelude::Client>) -> RateLimiter {
    match redis {
        Some(redis) => RateLimiter::new(
            Arc::new(RedisRateLimitBackend::new(redis.clone()).await),
            RateLimitConfig::default(),
        ),
        None => RateLimiter::default(),
    }
}

const REDIS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Once connected, a lost Redis fails calls (rate limiting and idempotency handle errors) rather
/// than queueing them until it reconnects.
const REDIS_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum RedisConnectError {
    #[error("REDIS_URL is invalid: {0}")]
    InvalidUrl(fred::error::Error),
    #[error("could not connect to REDIS_URL: {0}")]
    Unreachable(fred::error::Error),
    #[error("REDIS_URL did not answer within {}s", REDIS_CONNECT_TIMEOUT.as_secs())]
    Timeout,
}

/// The shared `fred::Client`, or `None` without `REDIS_URL` (in-memory fallbacks). A configured
/// Redis must be reachable: shared rate limits and idempotency can't silently go per-replica.
pub async fn connect_redis(
    config: &RedisConfig,
) -> Result<Option<fred::prelude::Client>, RedisConnectError> {
    let Some(url) = config.url.as_deref().filter(|u| !u.trim().is_empty()) else {
        return Ok(None);
    };
    let mut cfg = FredConfig::from_url(url).map_err(RedisConnectError::InvalidUrl)?;
    cfg.password = config
        .password
        .as_ref()
        .map(|p| p.expose_secret().to_string());

    let client = Builder::from_config(cfg)
        .with_connection_config(|c| c.connection_timeout = REDIS_CONNECT_TIMEOUT)
        .with_performance_config(|c| c.default_command_timeout = REDIS_COMMAND_TIMEOUT)
        .set_policy(ReconnectPolicy::new_exponential(0, 100, 30_000, 2))
        .build()
        .map_err(RedisConnectError::InvalidUrl)?;

    tokio::time::timeout(REDIS_CONNECT_TIMEOUT, client.init())
        .await
        .map_err(|_| RedisConnectError::Timeout)?
        .map_err(RedisConnectError::Unreachable)?;
    tracing::info!("Connected to Redis");
    Ok(Some(client))
}

fn register_pool_metrics(pool: &PgPool) {
    let pool = pool.clone();

    let gauge: ObservableGauge<u64> = GLOBAL_METER
        .u64_observable_gauge("meteroid_db_pool_status")
        .with_description("Database connection pool status")
        .with_callback(move |observer| {
            let status = pool.status();
            observer.observe(
                status.max_size as u64,
                &[KeyValue::new("state", "max_size")],
            );
            observer.observe(status.size as u64, &[KeyValue::new("state", "size")]);
            observer.observe(
                status.available as u64,
                &[KeyValue::new("state", "available")],
            );
            observer.observe(status.waiting as u64, &[KeyValue::new("state", "waiting")]);
        })
        .build();
    // Keep gauges alive for the process lifetime (one store per process)
    Box::leak(Box::new(gauge));
}

/// What every binary starts from: the core config, the edition, and the process's store,
/// metering client and outbound HTTP clients, built once.
pub struct Process {
    pub config: Config,
    pub modules: Modules,
    pub store: Store,
    pub usage_client: MeteringUsageClient,
    pub egress: Egress,
}

/// Loads the environment, then builds the edition: its settings are read from the environment.
pub fn start(edition: impl FnOnce() -> Modules) -> Result<Process, dotenvy::Error> {
    crate::load_dotenv()?;
    let config = Config::load();
    let modules = edition();
    let egress = egress(&config.webhook_out);

    common_logging::telemetry::init(&config.common.telemetry);
    Config::log_ignored_settings(&modules.edition.ignored_settings());

    let usage_client = MeteringUsageClient::connect(&config);
    let store = build_store(&modules, &config, &usage_client, &egress);
    Ok(Process {
        config,
        modules,
        store,
        usage_client,
        egress,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_egress_refuses_private_destinations() {
        let egress = egress(&WebhookOutConfig::default());
        for purpose in [
            EgressPurpose::Webhooks,
            EgressPurpose::Alerts,
            EgressPurpose::Sso,
        ] {
            let client = egress.client(purpose).expect("egress client");
            assert!(!client.allows_private_destinations(), "{purpose:?}");
        }
    }

    #[test]
    fn private_destinations_can_be_allowed() {
        let egress = egress(&WebhookOutConfig {
            allow_private_destinations: true,
            ..WebhookOutConfig::default()
        });
        let client = egress
            .client(EgressPurpose::Webhooks)
            .expect("egress client");
        assert!(client.allows_private_destinations());
    }

    #[test]
    fn without_redis_requests_are_not_limited() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        assert!(!runtime.block_on(rate_limiter(None)).is_enabled());
    }
}
