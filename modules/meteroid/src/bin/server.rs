use std::sync::Arc;
use tokio::signal;

use common_build_info::BuildInfo;
use common_logging::init::init_telemetry;
use meteroid::adapters::stripe::Stripe;
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::config::Config;
use meteroid::eventbus::setup_eventbus_handlers;
use meteroid::migrations;
use meteroid::services::storage::S3Storage;
use meteroid::{bootstrap, singletons};
use meteroid_store::Services;
use stripe_client::client::StripeClient;

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

    let store = singletons::get_store().await;

    let store_arc = Arc::new(store.clone());

    let services = Services::new(store_arc, Arc::new(MeteringUsageClient::get().clone()));

    migrations::run(&store.pool).await?;
    bootstrap::bootstrap_once(store.clone()).await?;
    setup_eventbus_handlers(store.clone(), config.clone()).await;

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let grpc_server = meteroid::api::server::start_api_server(
        config.clone(),
        store.clone(),
        services,
        object_store_service.clone(),
    );

    let exit = signal::ctrl_c();

    let stripe = Arc::new(StripeClient::new());
    let stripe_adapter = Arc::new(Stripe { client: stripe });

    let rest_server = meteroid::api_rest::server::start_rest_server(
        config,
        object_store_service.clone(),
        stripe_adapter.clone(),
        store.clone(),
    );

    tokio::select! {
        grpc_result = grpc_server => {
            if let Err(e) = grpc_result {
                log::error!("Error starting gRPC API server: {}", e);
            }
        },
        rest_result = rest_server => {
            if let Err(e) = rest_result {
                log::error!("Error starting REST API server: {}", e);
            }
        },
        _ = exit => {
              log::info!("Interrupted");
        }
    }

    Ok(())
}
