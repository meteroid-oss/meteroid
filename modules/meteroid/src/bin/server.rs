use std::sync::Arc;

use tokio::signal;

use common_build_info::BuildInfo;
use common_grpc::middleware::client::build_layered_client_service;
use common_logging::init::init_telemetry;
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use meteroid::adapters::stripe::Stripe;
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::config::Config;
use meteroid::eventbus::{create_eventbus_memory, setup_eventbus_handlers};
use meteroid::singletons::get_pool;
use meteroid::webhook_in_api;
use meteroid_migrations::migrations;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match dotenvy::dotenv() {
        Err(error) if error.not_found() => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Ok(()),
    }?;

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {}", build_info);

    let config = Config::get();

    init_telemetry(&config.common.telemetry, env!("CARGO_BIN_NAME"));

    let pool = get_pool();

    let metering_channel = tonic::transport::Channel::from_shared(config.metering_endpoint.clone())
        .expect("Invalid metering_endpoint")
        .connect_lazy();
    let metering_layered_channel =
        build_layered_client_service(metering_channel, &config.internal_auth);

    let query_service_client = UsageQueryServiceClient::new(metering_layered_channel.clone());
    let metering_service = MetersServiceClient::new(metering_layered_channel);

    // this creates a new pool, as it is incompatible with the one for cornucopia.
    let store = meteroid_store::Store::new(
        config.database_url.clone(),
        config.secrets_crypt_key.clone(),
        config.jwt_secret.clone(),
        create_eventbus_memory(),
        Arc::new(MeteringUsageClient::new(
            query_service_client,
            metering_service,
        )),
    )?;

    setup_eventbus_handlers(store.clone(), config.clone()).await;

    let private_server = meteroid::api::server::start_api_server(config.clone(), store.clone());

    let exit = signal::ctrl_c();

    let mut conn = meteroid::db::get_connection(&pool).await?;
    migrations::run_migrations(&mut **conn).await?;

    let object_store_client =
        Arc::new(object_store::parse_url(&url::Url::parse(&config.object_store_uri)?)?.0);

    let stripe_adapter = Arc::new(Stripe {
        client: stripe_client::client::StripeClient::new(),
    });

    tokio::select! {
        _ = private_server => {},
        _ = webhook_in_api::serve(
            config.invoicing_webhook_addr,
            object_store_client,
            stripe_adapter.clone(),
            store,
        ) => {},
        _ = exit => {
              log::info!("Interrupted");
        }
    }

    Ok(())
}
