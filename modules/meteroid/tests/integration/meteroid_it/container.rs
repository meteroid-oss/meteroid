use std::sync::Arc;
use std::time::Duration;

use diesel_async::SimpleAsyncConnection;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

use crate::helpers;
use meteroid::config::Config;
use meteroid::eventbus::{create_eventbus_memory, setup_eventbus_handlers};
use meteroid::migrations;
use meteroid::services::storage::in_memory_object_store;
use meteroid_store::compute::clients::usage::{MockUsageClient, UsageClient};
use meteroid_store::store::{PgPool, StoreConfig};

pub struct MeteroidSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub config: Config,
    pub store: meteroid_store::Store,
}

pub async fn start_meteroid_with_port(
    meteroid_port: u16,
    metering_port: u16,
    postgres_connection_string: String,
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
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

    let store = meteroid_store::Store::new(StoreConfig {
        database_url: config.database_url.clone(),
        crypt_key: config.secrets_crypt_key.clone(),
        jwt_secret: config.jwt_secret.clone(),
        multi_organization_enabled: config.multi_organization_enabled,
        eventbus: create_eventbus_memory(),
        usage_client,
        svix: None,
    })
    .expect("Could not create store");

    populate_postgres(&store.pool, seed_level).await;

    setup_eventbus_handlers(store.clone(), config.clone()).await;

    log::info!("Starting gRPC server {}", config.grpc_listen_addr);
    let private_server = meteroid::api::server::start_api_server(
        config.clone(),
        store.clone(),
        in_memory_object_store(),
    );

    let join_handle_meteroid = tokio::spawn(async move {
        tokio::select! {
            _ = private_server => {},
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
    )
    .await
}

// TODO check if that replaces terminate_meteroid
// impl Drop for MeteroidSetup {
//     fn drop(&mut self) {
//         self.token.cancel();
//         // wait synchronously on join_handle
//         futures::executor::block_on(&self.join_handle).unwrap();
//         log::info!("Stopped meteroid server");
//     }
// }

pub async fn terminate_meteroid(token: CancellationToken, join_handle: JoinHandle<()>) {
    token.cancel();
    join_handle.await.unwrap();

    log::info!("Stopped meteroid server");
}

pub async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default().with_tag("15.2").start().await.unwrap();

    let port = container.get_host_port_ipv4(5432).await.unwrap();

    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    log::info!("Started testcontainers PostgreSQL :{}", port);

    (container, connection_string)
}

pub async fn populate_postgres(pool: &PgPool, seed_level: SeedLevel) {
    migrations::run(pool).await.unwrap();

    let mut conn = pool.get().await.unwrap();

    for seed in seed_level.seeds() {
        let contents = std::fs::read_to_string(seed.path()).expect("Can't access seed file");
        conn.batch_execute(contents.as_str())
            .await
            .inspect_err(|_err| {
                eprintln!("Seed failed to apply : {}", seed.path());
            })
            .unwrap();
    }
}

#[allow(clippy::upper_case_acronyms)]
pub enum SeedLevel {
    MINIMAL,
    PRODUCT,
    PLANS,
    SUBSCRIPTIONS,
}

impl SeedLevel {
    fn seeds(&self) -> Vec<Seed> {
        match *self {
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

impl Seed {
    fn path(&self) -> &str {
        match *self {
            Seed::MINIMAL => "tests/data/0_minimal.sql",
            Seed::CUSTOMERS => "tests/data/1_customers.sql",
            Seed::METERS => "tests/data/1_meters.sql",
            Seed::PLANS => "tests/data/2_plans.sql",
            Seed::SUBSCRIPTIONS => "tests/data/3_subscriptions.sql",
        }
    }
}
