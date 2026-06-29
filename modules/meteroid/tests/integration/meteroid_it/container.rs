use crate::helpers;
use backon::{ConstantBuilder, Retryable};
use meteroid::adapters::stripe::Stripe;
use meteroid::config::Config;
use meteroid::eventbus::{create_eventbus_noop, setup_eventbus_handlers};
use meteroid::migrations;
use meteroid::services::storage::{ObjectStoreService, in_memory_object_store};
use meteroid_mailer::config::MailerConfig;
use meteroid_mailer::service::MailerService;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::Services;
use meteroid_store::clients::usage::{MockUsageClient, UsageClient};
use meteroid_store::store::{PgConfig, PgPool, StoreConfig};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use stripe_client::client::StripeClient;
use testcontainers::core::WaitFor;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, TestcontainersError};
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

/// Shared Postgres container and base connection string.
/// Migrations are run once on the template database.
static POSTGRES_INSTANCE: OnceCell<SharedPostgres> = OnceCell::const_new();
static TEST_DB_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Container ID for cleanup at process exit.
/// Rust never drops statics, so ContainerAsync::Drop never fires.
/// We register a C atexit handler to `docker rm -f` the container on normal exit.
/// The testcontainers `watchdog` feature handles signal-based termination (SIGINT/SIGTERM).
static CLEANUP_CONTAINER_ID: OnceLock<String> = OnceLock::new();

unsafe extern "C" {
    fn atexit(cb: extern "C" fn()) -> std::ffi::c_int;
}

extern "C" fn cleanup_postgres_container() {
    if let Some(id) = CLEANUP_CONTAINER_ID.get() {
        log::info!("Cleaning up test Postgres container {}", id);
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", id])
            .output();
    }
}

struct SharedPostgres {
    /// Held to keep the container alive; `None` when using an external Postgres
    /// supplied via the `METEROID_TEST_PG_URL` env var.
    #[allow(dead_code)]
    container: Option<ContainerAsync<GenericImage>>,
    base_connection_string: String,
}

/// Replace the database name (the URL path) on a Postgres connection string.
fn with_database(base_url: &str, db_name: &str) -> String {
    let (prefix, _) = base_url
        .rsplit_once('/')
        .expect("Postgres URL must contain a database path component");
    format!("{}/{}", prefix, db_name)
}

/// Initialize the shared Postgres (external or container-backed) and run migrations
/// once on the `meteroid_template` database.
async fn get_shared_postgres() -> &'static SharedPostgres {
    POSTGRES_INSTANCE
        .get_or_init(|| async {
            use diesel::sql_query;
            use diesel_async::RunQueryDsl;

            let (container, base_connection_string) =
                if let Ok(url) = std::env::var("METEROID_TEST_PG_URL") {
                    log::info!("Using external Postgres from METEROID_TEST_PG_URL");
                    (None, url)
                } else {
                    let container = start_postgres_container().await;
                    CLEANUP_CONTAINER_ID.set(container.id().to_string()).ok();
                    unsafe { atexit(cleanup_postgres_container) };
                    let port = container.get_host_port_ipv4(5432).await.unwrap();
                    let url = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);
                    (Some(container), url)
                };

            let pg_config = PgConfig::new(base_connection_string.clone());
            let pool = meteroid_store::store::diesel_make_pg_pool(pg_config)
                .expect("Failed to create pool");

            let mut conn = pool.get().await.unwrap();

            // Cross-process serialization: under nextest, every test is a fresh
            // process that races to create + migrate `meteroid_template`. Hold a
            // Postgres advisory lock so only one process performs the setup; others
            // wait, see the template exists, and skip straight to migrations (which
            // are idempotent via diesel's __diesel_schema_migrations).
            const TEMPLATE_INIT_LOCK: i64 = 0x6D65_7465_726F_6964; // "meteroid"
            sql_query(format!("SELECT pg_advisory_lock({})", TEMPLATE_INIT_LOCK))
                .execute(&mut conn)
                .await
                .unwrap();

            let template_count: i64 = diesel::dsl::sql::<diesel::sql_types::BigInt>(
                "SELECT count(*) FROM pg_database WHERE datname = 'meteroid_template'",
            )
            .get_result(&mut conn)
            .await
            .unwrap();

            if template_count == 0 {
                // First process in this run: reap orphan test DBs left by prior runs
                // (no-FORCE drop skips any DB still in use, harmlessly).
                let leftovers: Vec<String> = diesel::dsl::sql::<diesel::sql_types::Text>(
                    "SELECT datname FROM pg_database WHERE datname LIKE 'test_db_%'",
                )
                .load(&mut conn)
                .await
                .unwrap_or_default();
                for db in leftovers {
                    let _ = sql_query(format!("DROP DATABASE {}", db))
                        .execute(&mut conn)
                        .await;
                }

                sql_query("CREATE DATABASE meteroid_template")
                    .execute(&mut conn)
                    .await
                    .unwrap();
            }

            let template_url = with_database(&base_connection_string, "meteroid_template");
            let template_pool =
                meteroid_store::store::diesel_make_pg_pool(PgConfig::new(template_url))
                    .expect("Failed to create template pool");
            migrations::run(&template_pool).await.unwrap();

            sql_query(format!("SELECT pg_advisory_unlock({})", TEMPLATE_INIT_LOCK))
                .execute(&mut conn)
                .await
                .unwrap();

            log::info!("Shared Postgres ready with migrations on template");

            SharedPostgres {
                container,
                base_connection_string,
            }
        })
        .await
}

pub struct MeteroidSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub config: Config,
    pub store: meteroid_store::Store,
    pub services: Services,
    pub object_store: Arc<dyn ObjectStoreService>,
}

pub async fn start_meteroid_with_port(
    meteroid_port: u16,
    metering_port: u16,
    postgres_connection_string: String,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
) -> MeteroidSetup {
    let rest_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Could not bind REST listener");
    let rest_api_addr = rest_listener.local_addr().expect("Could not get REST addr");

    let config = super::config::mocked_config(
        postgres_connection_string,
        rest_api_addr,
        meteroid_port,
        metering_port,
    );

    start_meteroid_from_config(config, seed_level, usage_client, mailer, rest_listener).await
}

async fn start_meteroid_from_config(
    config: Config,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
    rest_listener: tokio::net::TcpListener,
) -> MeteroidSetup {
    let token = CancellationToken::new();
    let cloned_token = token.clone();
    let stripe = Arc::new(StripeClient::new());

    let store = meteroid_store::Store::new(StoreConfig {
        pg: config.pg.clone(),
        crypt_key: config.secrets_crypt_key.0.clone(),
        jwt_secret: config.jwt_secret.clone(),
        multi_organization_enabled: config.multi_organization_enabled,
        mailer_enabled: config.mailer_enabled(),
        public_url: config.public_url.clone(),
        eventbus: create_eventbus_noop(),
        mailer: mailer.clone(),
        oauth: meteroid_oauth::service::OauthServices::new(OauthConfig::dummy()),
        domains_whitelist: config.domains_whitelist(),
        billing: None,
        billing_default_plan_id: None,
        admin_organization_id: None,
        usage_client: usage_client.clone(),
        invite_ttl_days: 7,
    })
    .expect("Could not create store");

    let services = Services::new(Arc::new(store.clone()), usage_client, stripe);

    populate_postgres(&store.pool, seed_level).await;

    setup_eventbus_handlers(store.clone(), config.clone()).await;

    log::info!("Starting gRPC server {}", config.grpc_listen_addr);
    let private_server = meteroid::api::server::start_api_server(
        config.clone(),
        store.clone(),
        services.clone(),
        in_memory_object_store(),
        None,
        Arc::new(meteroid::services::svix_cache::NoopSvixEndpointCache),
    );

    let stripe = Arc::new(StripeClient::new());
    let stripe_adapter = Arc::new(Stripe { client: stripe });

    let object_store = in_memory_object_store();
    let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let rest_server = meteroid::api_rest::server::start_rest_server_with_listener(
        config.clone(),
        object_store.clone(),
        stripe_adapter,
        store.clone(),
        services.clone(),
        ready,
        rest_listener,
        None,
    );

    let join_handle_meteroid = tokio::spawn(async move {
        tokio::select! {
            _ = private_server => {},
            _ = rest_server => {},
            _ = cloned_token.cancelled() => {
                log::info!("Interrupted meteroid server via token");
            }
        }
    });

    wait_for_tcp_ready(config.grpc_listen_addr, Duration::from_secs(10)).await;

    let meteroid_endpoint = format!("http://{}", config.grpc_listen_addr);
    log::info!("Creating gRPC channel {}", meteroid_endpoint);
    let channel = Channel::from_shared(meteroid_endpoint)
        .expect("Invalid meteroid_endpoint")
        .connect_lazy();

    MeteroidSetup {
        token,
        join_handle: join_handle_meteroid,
        channel,
        config: config.clone(),
        store,
        services,
        object_store,
    }
}

pub async fn start_meteroid(
    postgres_connection_string: String,
    seed_level: SeedLevel,
) -> MeteroidSetup {
    let meteroid_port = helpers::network::free_local_port().expect("Could not get free port");
    let metering_port = helpers::network::free_local_port().expect("Could not get free port");

    start_meteroid_with_port(
        meteroid_port,
        metering_port,
        postgres_connection_string,
        seed_level,
        Arc::new(MockUsageClient::noop()),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
    )
    .await
}

pub async fn start_meteroid_with_clients(
    postgres_connection_string: String,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
) -> MeteroidSetup {
    let meteroid_port = helpers::network::free_local_port().expect("Could not get free port");
    let metering_port = helpers::network::free_local_port().expect("Could not get free port");

    start_meteroid_with_port(
        meteroid_port,
        metering_port,
        postgres_connection_string,
        seed_level,
        usage_client,
        mailer,
    )
    .await
}

impl Drop for MeteroidSetup {
    fn drop(&mut self) {
        self.token.cancel();
        self.join_handle.abort();
        log::info!("Stopped meteroid server  ");
    }
}

async fn wait_for_tcp_ready(addr: std::net::SocketAddr, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut backoff = Duration::from_millis(1);
    loop {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(_) => return,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_millis(50));
            }
            Err(e) => panic!(
                "gRPC server at {} did not become ready within {:?}: {}",
                addr, timeout, e
            ),
        }
    }
}

/// Start the raw Postgres container (internal use).
async fn start_postgres_container() -> ContainerAsync<GenericImage> {
    (|| async {
        let postgres = GenericImage::new("ghcr.io/meteroid-oss/meteroid-postgres", "18.3-standard")
            .with_wait_for(WaitFor::log(LogWaitStrategy::stdout(
                "database system is ready to accept connections",
            )))
            .with_wait_for(WaitFor::log(LogWaitStrategy::stderr(
                "database system is ready to accept connections",
            )))
            .with_env_var("POSTGRES_DB", "postgres")
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres");

        postgres.start().await
    })
    .retry(ConstantBuilder::default())
    .notify(|err: &TestcontainersError, dur: Duration| {
        log::warn!(
            "Retrying to start docker container {:?} after {:?}",
            err,
            dur
        );
    })
    .await
    .unwrap()
}

/// Create a new test database from the template.
/// Returns the connection string for the new database.
pub async fn create_test_database() -> String {
    use diesel::sql_query;
    use diesel_async::RunQueryDsl;

    let shared = get_shared_postgres().await;
    let test_id = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let db_name = format!("test_db_{}_{}", std::process::id(), test_id);

    let pg_config = PgConfig::new(shared.base_connection_string.clone());
    let pool =
        meteroid_store::store::diesel_make_pg_pool(pg_config).expect("Failed to create pool");
    let mut conn = pool.get().await.unwrap();
    sql_query(format!(
        "CREATE DATABASE {} TEMPLATE meteroid_template",
        db_name
    ))
    .execute(&mut conn)
    .await
    .unwrap();

    with_database(&shared.base_connection_string, &db_name)
}

#[allow(dead_code)]
pub async fn start_redis_container() -> ContainerAsync<GenericImage> {
    GenericImage::new("redis", "8-alpine")
        .with_wait_for(WaitFor::log(LogWaitStrategy::stdout(
            "Ready to accept connections",
        )))
        .start()
        .await
        .expect("Failed to start Redis container")
}

/// Legacy function for backwards compatibility.
/// Prefer using `create_test_database()` for new tests.
#[allow(dead_code)]
pub async fn start_postgres() -> (ContainerAsync<GenericImage>, String) {
    let container = start_postgres_container().await;
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);
    log::info!("Started testcontainers PostgreSQL :{}", port);
    (container, connection_string)
}

pub async fn populate_postgres(pool: &PgPool, seed_level: SeedLevel) {
    // Migrations are already applied on the template DB; cloned DBs inherit them.
    for seed in seed_level.seeds() {
        match seed {
            Seed::MINIMAL => crate::data::minimal::run_minimal_seed(pool).await,
            Seed::CUSTOMERS => crate::data::customers::run_customers_seed(pool).await,
            Seed::METERS => crate::data::meters::run_meters_seed(pool).await,
            Seed::PLANS => crate::data::plans::run_plans_seed(pool).await,
            Seed::SUBSCRIPTIONS => unimplemented!("Subscription seed level is not implemented"),
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
#[allow(dead_code)]
pub enum SeedLevel {
    NONE,
    MINIMAL,
    PRODUCT,
    PLANS,
    SUBSCRIPTIONS,
}

impl SeedLevel {
    fn seeds(&self) -> Vec<Seed> {
        match *self {
            SeedLevel::NONE => vec![],
            SeedLevel::MINIMAL => vec![Seed::MINIMAL],
            SeedLevel::PRODUCT => vec![Seed::MINIMAL, Seed::CUSTOMERS, Seed::METERS],
            SeedLevel::PLANS => vec![Seed::MINIMAL, Seed::CUSTOMERS, Seed::METERS, Seed::PLANS],
            SeedLevel::SUBSCRIPTIONS => vec![
                Seed::MINIMAL,
                Seed::CUSTOMERS,
                Seed::METERS,
                Seed::PLANS,
                Seed::SUBSCRIPTIONS,
            ],
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
pub enum Seed {
    MINIMAL,
    CUSTOMERS,
    METERS,
    PLANS,
    SUBSCRIPTIONS,
}
