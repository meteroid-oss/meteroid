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
use std::time::Duration;
use stripe_client::client::StripeClient;
use testcontainers::core::WaitFor;
use testcontainers::core::wait::LogWaitStrategy;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, TestcontainersError};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

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

    let services = Services::new(Arc::new(store.clone()), usage_client, None, stripe);

    populate_postgres(&store.pool, seed_level).await;

    setup_eventbus_handlers(store.clone(), config.clone()).await;

    log::info!("Starting gRPC server {}", config.grpc_listen_addr);
    let private_server = meteroid::api::server::start_api_server(
        config.clone(),
        store.clone(),
        services.clone(),
        in_memory_object_store(),
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

pub async fn start_postgres() -> (ContainerAsync<GenericImage>, String) {
    let container = (|| async {
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
    .unwrap();

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
