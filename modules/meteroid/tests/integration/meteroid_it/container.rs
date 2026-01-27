use crate::helpers;
use backon::{ConstantBuilder, Retryable};
use meteroid::adapters::stripe::Stripe;
use meteroid::config::Config;
use meteroid::eventbus::{create_eventbus_noop, setup_eventbus_handlers};
use meteroid::migrations;
use meteroid::services::storage::in_memory_object_store;
use meteroid_mailer::config::MailerConfig;
use meteroid_mailer::service::MailerService;
use meteroid_oauth::config::OauthConfig;
use meteroid_store::Services;
use meteroid_store::clients::usage::{MockUsageClient, UsageClient};
use meteroid_store::store::{PgPool, StoreConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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

struct SharedPostgres {
    #[allow(dead_code)]
    container: ContainerAsync<GenericImage>,
    base_connection_string: String,
}

/// Initialize the shared Postgres container and run migrations once.
async fn get_shared_postgres() -> &'static SharedPostgres {
    POSTGRES_INSTANCE
        .get_or_init(|| async {
            use diesel::sql_query;
            use diesel_async::RunQueryDsl;

            let container = start_postgres_container().await;
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            let base_connection_string =
                format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

            // Create template database and run migrations once
            let pool = meteroid_store::store::diesel_make_pg_pool(base_connection_string.clone())
                .expect("Failed to create pool");

            // Create the template database
            let mut conn = pool.get().await.unwrap();
            sql_query("CREATE DATABASE meteroid_template")
                .execute(&mut conn)
                .await
                .ok(); // Ignore if already exists

            // Run migrations on template
            let template_url = format!(
                "postgres://postgres:postgres@127.0.0.1:{}/meteroid_template",
                port
            );
            let template_pool = meteroid_store::store::diesel_make_pg_pool(template_url)
                .expect("Failed to create template pool");
            migrations::run(&template_pool).await.unwrap();

            log::info!("Shared Postgres container ready with migrations on template");

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
}

pub async fn start_meteroid_with_port(
    meteroid_port: u16,
    metering_port: u16,
    postgres_connection_string: String,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    mailer: Arc<dyn MailerService>,
) -> MeteroidSetup {
    let rest_api_addr = helpers::network::free_local_socket().expect("Could not get webhook addr");

    let config = super::config::mocked_config(
        postgres_connection_string,
        rest_api_addr,
        meteroid_port,
        metering_port,
    );

    let token = CancellationToken::new();
    let cloned_token = token.clone();
    let stripe = Arc::new(StripeClient::new());

    let store = meteroid_store::Store::new(StoreConfig {
        database_url: config.database_url.clone(),
        crypt_key: config.secrets_crypt_key.0.clone(),
        jwt_secret: config.jwt_secret.clone(),
        multi_organization_enabled: config.multi_organization_enabled,
        skip_email_validation: !config.mailer_enabled(),
        public_url: config.public_url.clone(),
        eventbus: create_eventbus_noop(),
        mailer: mailer.clone(),
        oauth: meteroid_oauth::service::OauthServices::new(OauthConfig::dummy()),
        domains_whitelist: config.domains_whitelist(),
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
    );

    let stripe = Arc::new(StripeClient::new());
    let stripe_adapter = Arc::new(Stripe { client: stripe });

    let rest_server = meteroid::api_rest::server::start_rest_server(
        config.clone(),
        in_memory_object_store(),
        stripe_adapter,
        store.clone(),
        services.clone(),
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

    tokio::time::sleep(Duration::from_secs(1)).await;

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

/// Start the raw Postgres container (internal use).
async fn start_postgres_container() -> ContainerAsync<GenericImage> {
    (|| async {
        let postgres = GenericImage::new("ghcr.io/meteroid-oss/meteroid-postgres", "17.4")
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
    let db_name = format!("test_db_{}", test_id);

    // Create new database from template
    let pool = meteroid_store::store::diesel_make_pg_pool(shared.base_connection_string.clone())
        .expect("Failed to create pool");
    let mut conn = pool.get().await.unwrap();
    sql_query(format!(
        "CREATE DATABASE {} TEMPLATE meteroid_template",
        db_name
    ))
    .execute(&mut conn)
    .await
    .unwrap();

    // Extract port from base connection string
    let port = shared
        .base_connection_string
        .split(':')
        .next_back()
        .unwrap()
        .split('/')
        .next()
        .unwrap();

    format!(
        "postgres://postgres:postgres@127.0.0.1:{}/{}",
        port, db_name
    )
}

/// Legacy function for backwards compatibility.
/// Prefer using `create_test_database()` for new tests.
pub async fn start_postgres() -> (ContainerAsync<GenericImage>, String) {
    let container = start_postgres_container().await;
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);
    log::info!("Started testcontainers PostgreSQL :{}", port);
    (container, connection_string)
}

pub async fn populate_postgres(pool: &PgPool, seed_level: SeedLevel) {
    migrations::run(pool).await.unwrap();

    // let mut conn = pool.get().await.unwrap();

    for seed in seed_level.seeds() {
        match seed {
            Seed::MINIMAL => {
                crate::data::minimal::run_minimal_seed(pool).await;
            }
            Seed::CUSTOMERS => {
                crate::data::customers::run_customers_seed(pool).await;
            }
            Seed::METERS => {
                crate::data::meters::run_meters_seed(pool).await;
            }
            Seed::PLANS => {
                crate::data::plans::run_plans_seed(pool).await;
            }
            Seed::SUBSCRIPTIONS => {
                unimplemented!("Subscription seed level is not implemented")
            }
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
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
