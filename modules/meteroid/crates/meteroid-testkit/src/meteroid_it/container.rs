use crate::helpers;
use backon::{ExponentialBuilder, Retryable};
use meteroid::config::Config;
use meteroid::eventbus::{create_eventbus_noop, setup_eventbus_handlers};
use meteroid::migrations;
use meteroid::module::Modules;
use meteroid::services::egress::{Egress, EgressPurpose};
use meteroid::services::storage::{ObjectStoreService, in_memory_object_store};
use meteroid_mailer::config::MailerConfig;
use meteroid_mailer::service::MailerService;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::Services;
use meteroid_store::clients::usage::{MockUsageClient, UsageClient};
use meteroid_store::hooks::InstanceEdition;
use meteroid_store::store::{PgConfig, PgPool, StoreConfig};
use rate_limiter::RateLimiter;
use std::fmt::{Debug, Display};
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::core::{Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
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
    fn kill(pid: i32, sig: i32) -> std::ffi::c_int;
}

/// Test databases are named `test_db_{pid}_{n}`; one whose process is gone is an orphan.
fn is_orphan(db_name: &str) -> bool {
    db_name
        .split('_')
        .nth(2)
        .and_then(|pid| pid.parse::<i32>().ok())
        .is_some_and(|pid| unsafe { kill(pid, 0) } != 0)
}

/// A database cloned from the template. Dropping the handle drops the database,
/// so keep it alive for as long as anything is connected.
pub struct TestDatabase {
    pub url: String,
    name: String,
    base_url: String,
    /// Why this test's data may break the credit invariants; checked on drop unless set.
    invariants_opt_out: OnceLock<&'static str>,
    /// The seed the database was cloned with, so starting meteroid does not seed it again.
    seeded: Option<SeedLevel>,
}

impl TestDatabase {
    fn drop_database(&self) -> Result<(), Box<dyn std::error::Error>> {
        use diesel::{Connection, RunQueryDsl, sql_query};

        let mut conn = diesel::PgConnection::establish(&self.base_url)?;
        // DROP ... WITH (FORCE) rechecks the sessions it terminates every 100ms; the test's pool
        // still holds some, so they are closed and waited for here first.
        sql_query(format!(
            "ALTER DATABASE {} WITH ALLOW_CONNECTIONS false",
            self.name
        ))
        .execute(&mut conn)?;
        let sessions = format!(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{}'",
            self.name
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while sql_query(&sessions).execute(&mut conn)? > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        sql_query(format!("DROP DATABASE {} WITH (FORCE)", self.name)).execute(&mut conn)?;
        Ok(())
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let violations = if std::thread::panicking() || self.invariants_opt_out.get().is_some() {
            Vec::new()
        } else {
            helpers::invariants::database_violations(&self.url)
        };
        if let Err(err) = self.drop_database() {
            log::warn!("Could not drop test database {}: {err}", self.name);
        }
        assert!(
            violations.is_empty(),
            "credit invariants violated at the end of the test:\n{}",
            violations.join("\n")
        );
    }
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

            // Under nextest every test is its own process. The template is stamped with the
            // migration set once migrated, so only a process seeing a stale stamp takes the
            // lock: a global lock per process would serialize the whole run.
            let fingerprint = migrations_fingerprint();
            if template_fingerprint(&mut conn).await.as_deref() != Some(fingerprint.as_str()) {
                const TEMPLATE_INIT_LOCK: i64 = 0x6D65_7465_726F_6964; // "meteroid"
                sql_query(format!("SELECT pg_advisory_lock({})", TEMPLATE_INIT_LOCK))
                    .execute(&mut conn)
                    .await
                    .unwrap();

                if template_fingerprint(&mut conn).await.as_deref() != Some(fingerprint.as_str()) {
                    let template_count: i64 = diesel::dsl::sql::<diesel::sql_types::BigInt>(
                        "SELECT count(*) FROM pg_database WHERE datname = 'meteroid_template'",
                    )
                    .get_result(&mut conn)
                    .await
                    .unwrap();
                    if template_count == 0 {
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
                    // CREATE DATABASE ... TEMPLATE waits on any session still in the template.
                    drop(template_pool);

                    sql_query(format!(
                        "COMMENT ON DATABASE meteroid_template IS '{fingerprint}'"
                    ))
                    .execute(&mut conn)
                    .await
                    .unwrap();
                }

                sql_query(format!("SELECT pg_advisory_unlock({})", TEMPLATE_INIT_LOCK))
                    .execute(&mut conn)
                    .await
                    .unwrap();
            }

            reap_orphan_databases(&mut conn).await;

            log::info!("Shared Postgres ready with migrations on template");

            SharedPostgres {
                container,
                base_connection_string,
            }
        })
        .await
}

/// Hash of the migration directory names: diesel tracks applied migrations by version, so a
/// template migrated from the same set is up to date.
fn migrations_fingerprint() -> String {
    use sha2::{Digest, Sha256};

    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations/diesel");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("Could not read the migrations directory")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let digest = Sha256::digest(names.join("\n").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

async fn template_fingerprint(conn: &mut meteroid_store::store::PgConn) -> Option<String> {
    database_fingerprint(conn, "meteroid_template").await
}

async fn database_fingerprint(
    conn: &mut meteroid_store::store::PgConn,
    database: &str,
) -> Option<String> {
    use diesel_async::RunQueryDsl;

    diesel::dsl::sql::<diesel::sql_types::Nullable<diesel::sql_types::Text>>(&format!(
        "SELECT shobj_description(oid, 'pg_database') FROM pg_database \
         WHERE datname = '{database}'"
    ))
    .get_result::<Option<String>>(conn)
    .await
    .ok()
    .flatten()
}

/// Drops test databases left behind by killed processes (a normal exit drops its own). One
/// process reaps at a time; the others go on to their test.
async fn reap_orphan_databases(conn: &mut meteroid_store::store::PgConn) {
    use diesel::sql_query;
    use diesel_async::RunQueryDsl;

    const REAP_LOCK: i64 = 0x6D65_7465_7265_6170; // "metereap"
    let locked: bool = diesel::dsl::sql::<diesel::sql_types::Bool>(&format!(
        "SELECT pg_try_advisory_lock({REAP_LOCK})"
    ))
    .get_result(conn)
    .await
    .unwrap_or(false);
    if !locked {
        return;
    }

    let leftovers: Vec<String> = diesel::dsl::sql::<diesel::sql_types::Text>(
        "SELECT datname FROM pg_database WHERE datname LIKE 'test_db_%'",
    )
    .load(conn)
    .await
    .unwrap_or_default();
    for db in leftovers.iter().filter(|db| is_orphan(db)) {
        let _ = sql_query(format!("DROP DATABASE {} WITH (FORCE)", db))
            .execute(conn)
            .await;
    }

    let _ = sql_query(format!("SELECT pg_advisory_unlock({REAP_LOCK})"))
        .execute(conn)
        .await;
}

pub struct MeteroidSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub config: Config,
    pub store: meteroid_store::Store,
    pub services: Services,
    pub object_store: Arc<dyn ObjectStoreService>,
    _database: TestDatabase,
}

/// Start meteroid on an already-bound gRPC listener that the caller holds from
/// the moment the port is selected. This is the cross-wired path: the metering
/// server is configured to reach meteroid at this port, so the caller binds the
/// listener first (via [`bind_meteroid_grpc`]), reads its port for the metering
/// config, then hands the still-bound listener here — closing the TOCTOU window
/// entirely, since the port is never released between selection and `serve`.
pub async fn start_meteroid_with_grpc_listener(
    grpc_listener: tokio::net::TcpListener,
    metering_port: u16,
    database: TestDatabase,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
) -> MeteroidSetup {
    start_meteroid_with_grpc_listener_and_modules(
        grpc_listener,
        metering_port,
        database,
        seed_level,
        usage_client,
        mailer,
        Modules::core(),
    )
    .await
}

/// Like [`start_meteroid_with_grpc_listener`], serving the edition `modules` too.
pub async fn start_meteroid_with_grpc_listener_and_modules(
    grpc_listener: tokio::net::TcpListener,
    metering_port: u16,
    database: TestDatabase,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
    modules: Modules,
) -> MeteroidSetup {
    let meteroid_addr = grpc_listener.local_addr().expect("Could not get gRPC addr");
    let rest_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Could not bind REST listener");
    let rest_api_addr = rest_listener.local_addr().expect("Could not get REST addr");

    let config = super::config::mocked_config(
        database.url.clone(),
        rest_api_addr,
        meteroid_addr.port(),
        metering_port,
    );

    let grpc_incoming = tonic::transport::server::TcpIncoming::from(grpc_listener);

    start_meteroid_from_config(
        config,
        database,
        seed_level,
        usage_client,
        mailer,
        rest_listener,
        grpc_incoming,
        Default::default(),
        None,
        modules,
    )
    .await
}

/// Bind a held gRPC listener for meteroid. Callers that cross-wire the port into
/// a separately-started server (e.g. metering) bind here first, read
/// `local_addr().port()` for that server's config, then pass the listener to
/// [`start_meteroid_with_grpc_listener`] without ever releasing the port.
pub async fn bind_meteroid_grpc() -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Could not bind gRPC listener")
}

#[allow(clippy::too_many_arguments)]
async fn start_meteroid_from_config(
    config: Config,
    database: TestDatabase,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
    rest_listener: tokio::net::TcpListener,
    grpc_incoming: tonic::transport::server::TcpIncoming,
    rate_limiter: RateLimiter,
    instance: Option<Arc<dyn InstanceEdition>>,
    modules: Modules,
) -> MeteroidSetup {
    let token = CancellationToken::new();
    let cloned_token = token.clone();

    meteroid_middleware::server::auth::strategies::jwt_strategy::invalidate_all_member_access_cache()
        .await;

    let egress = Egress::new(config.webhook_out.clone(), modules.destination_guard());

    let store = meteroid_store::Store::new(StoreConfig {
        pg: config.pg.clone(),
        crypt_key: config.secrets_crypt_key.0.clone(),
        jwt_secret: config.jwt_secret.clone(),
        multi_organization_enabled: config.multi_organization_enabled,
        mailer_enabled: config.mailer_enabled(),
        public_url: config.public_url.clone(),
        rest_api_external_url: config.rest_api_external_url.clone(),
        webhook_external_url: config.webhook_external_url.clone(),
        eventbus: create_eventbus_noop(),
        mailer: mailer.clone(),
        oauth: meteroid_oauth::service::OauthServices::new(
            OauthConfig::dummy(),
            egress
                .client(EgressPurpose::Sso)
                .expect("Could not build the SSO egress client"),
        ),
        domains_whitelist: config.domains_whitelist(),
        organization_hooks: modules.edition.organization_hooks(&config),
        instance: instance.unwrap_or_else(|| modules.edition.instance()),
        hooks: modules.store_hooks(),
        admin_organization_id: config.admin_organization_id,
        usage_client: usage_client.clone(),
        invite_ttl_days: 7,
        mfa_issuer: "Meteroid".to_string(),
    })
    .expect("Could not create store");

    let services = Services::new(Arc::new(store.clone()), usage_client);

    match database.seeded {
        None => populate_postgres(&store.pool, seed_level).await,
        Some(seeded) if seeded == seed_level => {}
        Some(seeded) => panic!("the database holds the {seeded:?} seed, not {seed_level:?}"),
    }

    setup_eventbus_handlers(store.clone(), config.clone(), &modules).await;

    log::info!("Starting gRPC server {}", config.api.grpc_listen_addr);
    let private_server = meteroid::api::server::start_api_server_with_incoming(
        config.clone(),
        store.clone(),
        services.clone(),
        in_memory_object_store(),
        rate_limiter.clone(),
        None,
        grpc_incoming,
        modules.clone(),
        egress.clone(),
        std::sync::Arc::new(meteroid::api_rest::webhook_catalog::RestWebhookCatalog),
    );

    let object_store = in_memory_object_store();
    let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    // Built on the first connection: most tests never call the REST API, and building it (the
    // OpenAPI documents above all) is most of a server start.
    let rest_server = {
        let (config, object_store, store, services) = (
            config.clone(),
            object_store.clone(),
            store.clone(),
            services.clone(),
        );
        let mut rest_listener = rest_listener;
        async move {
            let first = axum::serve::Listener::accept(&mut rest_listener).await;
            meteroid::api_rest::server::start_rest_server_with_listener(
                config,
                object_store,
                store,
                services,
                ready,
                FirstConnection {
                    first: Some(first),
                    listener: rest_listener,
                },
                rate_limiter,
                modules,
                egress,
            )
            .await
        }
    };

    let join_handle_meteroid = tokio::spawn(async move {
        tokio::select! {
            _ = private_server => {},
            _ = rest_server => {},
            _ = cloned_token.cancelled() => {
                log::info!("Interrupted meteroid server via token");
            }
        }
    });

    wait_for_tcp_ready(config.api.grpc_listen_addr, Duration::from_secs(10)).await;

    let meteroid_endpoint = format!("http://{}", config.api.grpc_listen_addr);
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
        _database: database,
    }
}

/// Start meteroid holding its own gRPC listener from selection through `serve`,
/// eliminating the TOCTOU race that plagues `free_local_port` (which releases
/// the ephemeral port before it is re-bound). Used by every path that does not
/// cross-wire the port into a separately-started metering server.
#[allow(clippy::too_many_arguments)]
async fn start_meteroid_holding_listener(
    database: TestDatabase,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
    rate_limiter: RateLimiter,
    rate_limit_config: Option<meteroid::config::RateLimitConfig>,
    admin_organization_id: Option<common_domain::ids::OrganizationId>,
    instance: Option<Arc<dyn InstanceEdition>>,
    modules: Modules,
) -> MeteroidSetup {
    let grpc_listener = bind_meteroid_grpc().await;
    let meteroid_addr = grpc_listener.local_addr().expect("Could not get gRPC addr");
    let rest_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Could not bind REST listener");
    let rest_api_addr = rest_listener.local_addr().expect("Could not get REST addr");

    // The metering endpoint is never bound on this path (mock/no-op usage
    // client), so an ephemeral port number for it is sufficient.
    let metering_port = helpers::network::free_local_port().expect("Could not get free port");

    let mut config = super::config::mocked_config(
        database.url.clone(),
        rest_api_addr,
        meteroid_addr.port(),
        metering_port,
    );
    if let Some(rate_limit_config) = rate_limit_config {
        config.api.rate_limit = rate_limit_config;
    }
    config.admin_organization_id = admin_organization_id;

    let grpc_incoming = tonic::transport::server::TcpIncoming::from(grpc_listener);

    start_meteroid_from_config(
        config,
        database,
        seed_level,
        usage_client,
        mailer,
        rest_listener,
        grpc_incoming,
        rate_limiter,
        instance,
        modules,
    )
    .await
}

pub async fn start_meteroid_with_rate_limiter(
    database: TestDatabase,
    seed_level: SeedLevel,
    rate_limiter: RateLimiter,
    rate_limit_config: meteroid::config::RateLimitConfig,
) -> MeteroidSetup {
    start_meteroid_holding_listener(
        database,
        seed_level,
        Arc::new(MockUsageClient::noop()),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        rate_limiter,
        Some(rate_limit_config),
        None,
        None,
        Modules::core(),
    )
    .await
}

/// Like [`start_meteroid`], as the given installation and serving the edition `modules`.
pub async fn start_meteroid_with_modules(
    database: TestDatabase,
    seed_level: SeedLevel,
    instance: Arc<dyn InstanceEdition>,
    modules: Modules,
) -> MeteroidSetup {
    start_meteroid_holding_listener(
        database,
        seed_level,
        Arc::new(MockUsageClient::noop()),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        Default::default(),
        None,
        None,
        Some(instance),
        modules,
    )
    .await
}

pub async fn start_meteroid(database: TestDatabase, seed_level: SeedLevel) -> MeteroidSetup {
    start_meteroid_holding_listener(
        database,
        seed_level,
        Arc::new(MockUsageClient::noop()),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        Default::default(),
        None,
        None,
        None,
        Modules::core(),
    )
    .await
}

/// Like [`start_meteroid`], with the seeded organization as the platform admin org.
/// The seeded organization is the platform admin organization; `modules` serve the admin APIs.
pub async fn start_meteroid_as_platform_admin(
    database: TestDatabase,
    seed_level: SeedLevel,
    modules: Modules,
) -> MeteroidSetup {
    start_meteroid_holding_listener(
        database,
        seed_level,
        Arc::new(MockUsageClient::noop()),
        meteroid_mailer::service::mailer_service(MailerConfig::dummy()),
        Default::default(),
        None,
        Some(crate::data::ids::ORGANIZATION_ID),
        None,
        modules,
    )
    .await
}

pub async fn start_meteroid_with_clients(
    database: TestDatabase,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
) -> MeteroidSetup {
    start_meteroid_holding_listener(
        database,
        seed_level,
        usage_client,
        mailer,
        Default::default(),
        None,
        None,
        None,
        Modules::core(),
    )
    .await
}

/// Like [`start_meteroid_with_clients`], serving the edition `modules` too.
pub async fn start_meteroid_with_clients_and_modules(
    database: TestDatabase,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
    modules: Modules,
) -> MeteroidSetup {
    start_meteroid_holding_listener(
        database,
        seed_level,
        usage_client,
        mailer,
        Default::default(),
        None,
        None,
        None,
        modules,
    )
    .await
}

impl MeteroidSetup {
    /// Skips the credit invariants checked when the test's database is dropped. Only for a test
    /// that writes states the product cannot reach (raw SQL fixtures); `reason` says which.
    #[allow(dead_code)]
    pub fn skip_credit_invariants(&self, reason: &'static str) {
        self._database.skip_credit_invariants(reason);
    }
}

impl TestDatabase {
    #[allow(dead_code)]
    pub fn skip_credit_invariants(&self, reason: &'static str) {
        let _ = self.invariants_opt_out.set(reason);
    }
}

impl Drop for MeteroidSetup {
    fn drop(&mut self) {
        self.token.cancel();
        self.join_handle.abort();
        log::info!("Stopped meteroid server  ");
    }
}

/// A listener whose first connection was already accepted.
struct FirstConnection {
    first: Option<(tokio::net::TcpStream, std::net::SocketAddr)>,
    listener: tokio::net::TcpListener,
}

impl axum::serve::Listener for FirstConnection {
    type Io = tokio::net::TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.first.take() {
            Some(first) => first,
            None => axum::serve::Listener::accept(&mut self.listener).await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
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
    start_with_retry("start Postgres container", || async {
        let postgres = GenericImage::new("ghcr.io/meteroid-oss/meteroid-postgres", "18.3-standard")
            .with_wait_for(WaitFor::log(LogWaitStrategy::stdout(
                "database system is ready to accept connections",
            )))
            .with_wait_for(WaitFor::log(LogWaitStrategy::stderr(
                "database system is ready to accept connections",
            )))
            .with_env_var("POSTGRES_DB", "postgres")
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres")
            // Throwaway DB: skip fsyncs, which otherwise saturate the host disk when tests run in parallel
            .with_env_var("POSTGRES_INITDB_ARGS", "--no-sync")
            // Each test clones a template of ~1,200 relation files: ~6x faster in memory.
            .with_mount(Mount::tmpfs_mount("/var/lib/postgresql"))
            .with_cmd([
                "postgres",
                "-c",
                "fsync=off",
                "-c",
                "synchronous_commit=off",
                "-c",
                "full_page_writes=off",
            ]);

        postgres.start().await
    })
    .await
}

/// Builds and starts a container, retrying when the image pull or Docker connection drops
/// mid-way: the container never came up, so `build` (a fresh builder each call) is safe to rerun.
pub async fn start_with_retry<T, E, F, Fut>(what: &str, build: F) -> T
where
    E: Display + Debug,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    build
        .retry(
            ExponentialBuilder::default()
                .with_min_delay(Duration::from_secs(2))
                .with_max_times(4),
        )
        .when(is_transient_start_error)
        .notify(|e: &E, delay: Duration| {
            log::warn!("{what}: transient container start error ({e}); retrying in {delay:?}");
        })
        .await
        .unwrap_or_else(|e| panic!("{what}: {e:?}"))
}

pub fn is_transient_start_error<E: Display + Debug>(e: &E) -> bool {
    let text = format!("{e} {e:?}");
    [
        "PullImage",
        "bytes remaining on stream",
        "failed to pull",
        "error trying to connect",
        "connection reset",
        "connection closed",
        "timed out",
        "EOF",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Create a new test database from the template.
pub async fn create_test_database() -> TestDatabase {
    let shared = get_shared_postgres().await;
    clone_database(shared, "meteroid_template", None).await
}

/// A database holding `seed_level`'s data: a clone of a template seeded once per run, which
/// [`start_meteroid`] and the other starters do not seed again.
pub async fn create_seeded_test_database(seed_level: SeedLevel) -> TestDatabase {
    let shared = get_shared_postgres().await;
    let Some(template) = seed_level.template() else {
        return clone_database(shared, "meteroid_template", None).await;
    };
    ensure_seeded_template(shared, seed_level, template).await;
    clone_database(shared, template, Some(seed_level)).await
}

async fn clone_database(
    shared: &SharedPostgres,
    template: &str,
    seeded: Option<SeedLevel>,
) -> TestDatabase {
    use diesel::sql_query;
    use diesel_async::RunQueryDsl;

    let test_id = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let db_name = format!("test_db_{}_{}", std::process::id(), test_id);

    let pg_config = PgConfig::new(shared.base_connection_string.clone());
    let pool =
        meteroid_store::store::diesel_make_pg_pool(pg_config).expect("Failed to create pool");
    let mut conn = pool.get().await.unwrap();
    sql_query(format!("CREATE DATABASE {db_name} TEMPLATE {template}"))
        .execute(&mut conn)
        .await
        .unwrap();

    TestDatabase {
        url: with_database(&shared.base_connection_string, &db_name),
        name: db_name,
        base_url: shared.base_connection_string.clone(),
        invariants_opt_out: OnceLock::new(),
        seeded,
    }
}

/// Builds `template` from the migrated template and `seed_level`'s seed unless it is already
/// stamped with the current migrations and seed sources.
async fn ensure_seeded_template(shared: &SharedPostgres, seed_level: SeedLevel, template: &str) {
    use diesel::sql_query;
    use diesel_async::RunQueryDsl;

    let fingerprint = seeded_fingerprint(seed_level);
    let pool = meteroid_store::store::diesel_make_pg_pool(PgConfig::new(
        shared.base_connection_string.clone(),
    ))
    .expect("Failed to create pool");
    let mut conn = pool.get().await.unwrap();
    if database_fingerprint(&mut conn, template).await.as_deref() == Some(fingerprint.as_str()) {
        return;
    }

    let lock = SEEDED_TEMPLATE_LOCK + seed_level as i64;
    sql_query(format!("SELECT pg_advisory_lock({lock})"))
        .execute(&mut conn)
        .await
        .unwrap();
    if database_fingerprint(&mut conn, template).await.as_deref() != Some(fingerprint.as_str()) {
        sql_query(format!("DROP DATABASE IF EXISTS {template} WITH (FORCE)"))
            .execute(&mut conn)
            .await
            .unwrap();
        sql_query(format!(
            "CREATE DATABASE {template} TEMPLATE meteroid_template"
        ))
        .execute(&mut conn)
        .await
        .unwrap();
        let template_pool = meteroid_store::store::diesel_make_pg_pool(PgConfig::new(
            with_database(&shared.base_connection_string, template),
        ))
        .expect("Failed to create template pool");
        populate_postgres(&template_pool, seed_level).await;
        // CREATE DATABASE ... TEMPLATE waits on any session still in the template.
        drop(template_pool);
        sql_query(format!("COMMENT ON DATABASE {template} IS '{fingerprint}'"))
            .execute(&mut conn)
            .await
            .unwrap();
    }
    sql_query(format!("SELECT pg_advisory_unlock({lock})"))
        .execute(&mut conn)
        .await
        .unwrap();
}

const SEEDED_TEMPLATE_LOCK: i64 = 0x6D65_7465_7365_6400; // "meteseed"

/// The migrations plus the seed sources: a seeded template is current while neither changed.
fn seeded_fingerprint(seed_level: SeedLevel) -> String {
    use sha2::{Digest, Sha256};

    const SEED_SOURCES: [&str; 6] = [
        include_str!("../data/ids.rs"),
        include_str!("../data/minimal.rs"),
        include_str!("../data/customers.rs"),
        include_str!("../data/meters.rs"),
        include_str!("../data/plans.rs"),
        include_str!("container.rs"),
    ];
    let mut hasher = Sha256::new();
    hasher.update(migrations_fingerprint());
    hasher.update(format!("{seed_level:?}"));
    for source in SEED_SOURCES {
        hasher.update(source);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[allow(dead_code)]
pub async fn start_redis_container() -> ContainerAsync<GenericImage> {
    start_with_retry("start Redis container", || {
        GenericImage::new("redis", "8-alpine")
            .with_wait_for(WaitFor::log(LogWaitStrategy::stdout(
                "Ready to accept connections",
            )))
            .start()
    })
    .await
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedLevel {
    NONE,
    MINIMAL,
    PRODUCT,
    PLANS,
    SUBSCRIPTIONS,
}

impl SeedLevel {
    fn template(self) -> Option<&'static str> {
        match self {
            SeedLevel::NONE | SeedLevel::SUBSCRIPTIONS => None,
            SeedLevel::MINIMAL => Some("meteroid_template_minimal"),
            SeedLevel::PRODUCT => Some("meteroid_template_product"),
            SeedLevel::PLANS => Some("meteroid_template_plans"),
        }
    }

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
