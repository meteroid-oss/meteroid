use std::time::Duration;

use deadpool_postgres::Pool;
use testcontainers::clients::Cli;
use testcontainers::{Container, RunnableImage};

use testcontainers_modules::postgres::Postgres;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;

use meteroid::config::Config;
use meteroid::eventbus::{create_eventbus_memory, setup_store_eventbus};
use meteroid_repository::migrations;

use crate::helpers;

pub struct MeteroidSetup {
    pub token: CancellationToken,
    pub join_handle: JoinHandle<()>,
    pub channel: Channel,
    pub config: Config,
    pub pool: Pool,
    pub store: meteroid_store::Store,
}

pub async fn start_meteroid_with_port(
    meteroid_port: u16,
    metering_port: u16,
    postgres_connection_string: String,
    seed_level: SeedLevel,
) -> MeteroidSetup {
    let invoicing_webhook_addr =
        helpers::network::free_local_socket().expect("Could not get webhook addr");

    let config = super::config::mocked_config(
        postgres_connection_string,
        invoicing_webhook_addr,
        meteroid_port,
        metering_port,
    );

    let pool = meteroid_repository::create_pool(&config.database_url);

    populate_postgres(pool.clone(), seed_level).await;

    let token = CancellationToken::new();
    let cloned_token = token.clone();

    let store = meteroid_store::Store::new(
        config.database_url.clone(),
        config.secrets_crypt_key.clone(),
        config.jwt_secret.clone(),
        create_eventbus_memory(),
    )
    .expect("Could not create store");

    setup_store_eventbus(store.clone(), config.clone()).await;

    log::info!("Starting gRPC server {}", config.listen_addr);
    let private_server =
        meteroid::api::server::start_api_server(config.clone(), pool.clone(), store.clone());

    let join_handle_meteroid = tokio::spawn(async move {
        tokio::select! {
            _ = private_server => {},
            _ = cloned_token.cancelled() => {
                log::info!("Interrupted meteroid server via token");
            }
        }
    });

    tokio::time::sleep(Duration::from_secs(1)).await;

    let meteroid_endpoint = format!("http://{}", config.listen_addr);

    log::info!("Creating gRPC channel {}", meteroid_endpoint);

    let channel = Channel::from_shared(meteroid_endpoint)
        .expect("Invalid meteroid_endpoint")
        .connect_lazy();

    MeteroidSetup {
        token: token,
        join_handle: join_handle_meteroid,
        channel: channel,
        config: config.clone(),
        pool: pool,
        store: store,
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

pub fn start_postgres<'a>(docker: &'a Cli) -> (Container<'a, Postgres>, String) {
    let image = RunnableImage::from(Postgres::default()).with_tag("15.2");
    let container = docker.run(image);

    let port = container.get_host_port_ipv4(5432);

    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    log::info!("Started testcontainers PostgreSQL :{}", port);

    (container, connection_string)
}

pub async fn populate_postgres(pool: Pool, seed_level: SeedLevel) {
    let mut conn = meteroid::db::get_connection(&pool).await.unwrap();

    migrations::run_migrations(&mut **conn).await.unwrap();

    for seed in seed_level.seeds() {
        let contents = std::fs::read_to_string(seed.path()).expect("Can't access seed file");
        conn.batch_execute(contents.as_str())
            .await
            .map_err(|err| {
                eprintln!("Seed failed to apply : {}", seed.path());
                err
            })
            .unwrap();
    }
}

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
