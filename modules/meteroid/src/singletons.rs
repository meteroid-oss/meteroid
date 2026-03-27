use crate::config::{Config, RedisConfig};
use crate::eventbus::{create_eventbus_memory, setup_eventbus_handlers};
use common_logging::GLOBAL_METER;
use fred::prelude::{Builder, ClientLike, Config as FredConfig, ReconnectPolicy};
use meteroid_store::Store;
use meteroid_store::store::{PgPool, StoreConfig};
use opentelemetry::KeyValue;
use opentelemetry::metrics::ObservableGauge;
use secrecy::ExposeSecret;

static STORE: tokio::sync::OnceCell<Store> = tokio::sync::OnceCell::const_new();

pub async fn get_store() -> &'static Store {
    STORE
        .get_or_init(|| async {
            let config = Config::get();

            let mailer = meteroid_mailer::service::mailer_service(config.mailer.clone());
            let oauth = meteroid_oauth::service::OauthServices::new(config.oauth.clone());

            let store = Store::new(StoreConfig {
                pg: config.pg.clone(),
                crypt_key: config.secrets_crypt_key.0.clone(),
                jwt_secret: config.jwt_secret.clone(),
                multi_organization_enabled: config.multi_organization_enabled,
                skip_email_validation: !config.mailer_enabled(),
                public_url: config.public_url.clone(),
                eventbus: create_eventbus_memory(),
                mailer,
                oauth,
                domains_whitelist: config.domains_whitelist(),
                billing: None,
                billing_default_plan_id: None,
                admin_organization_id: None,
            })
            .expect("Failed to initialize store");

            register_pool_metrics(&store.pool);

            setup_eventbus_handlers(store.clone(), config.clone()).await;

            store
        })
        .await
}

/// Connect to Redis and return a shared `fred::Client` handle.
/// Returns `None` if no URL is configured or the connection fails (non-fatal).
pub fn connect_redis(config: &RedisConfig) -> Option<fred::prelude::Client> {
    let url = config.url.as_deref()?;
    let mut cfg = match FredConfig::from_url(url) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Invalid Redis URL: {e}");
            return None;
        }
    };
    cfg.password = config
        .password
        .as_ref()
        .map(|p| p.expose_secret().to_string());

    let client = match Builder::from_config(cfg)
        .set_policy(ReconnectPolicy::new_exponential(0, 100, 30_000, 2))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to build Redis client: {e}");
            return None;
        }
    };

    client.connect();
    tracing::info!("Redis client started (connecting in background)");
    Some(client)
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
    // Keep gauges alive for app lifetime (called once for singleton pool)
    Box::leak(Box::new(gauge));
}
