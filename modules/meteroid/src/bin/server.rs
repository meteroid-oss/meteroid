use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::signal;

use common_build_info::BuildInfo;
use common_logging::telemetry;
use meteroid::adapters::stripe::Stripe;
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::config::Config;
use meteroid::eventbus::setup_eventbus_handlers;
use meteroid::migrations;
use meteroid::services::storage::S3Storage;
use meteroid::singletons::connect_redis;
use meteroid::svix::wire_svix;
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
    println!("Starting {build_info}");

    let config = Config::get();

    telemetry::init(&config.common.telemetry);

    let store = singletons::get_store().await;

    let stripe = Arc::new(StripeClient::new());

    let store_arc = Arc::new(store.clone());

    let services = Services::new(
        store_arc,
        Arc::new(MeteringUsageClient::get().clone()),
        stripe,
    );

    let fred_client = connect_redis(&config.redis);
    let redis_available = fred_client.is_some();

    let svix_wiring = wire_svix(&config.svix, fred_client);

    migrations::run(&store.pool).await?;
    bootstrap::bootstrap_once(store.clone(), svix_wiring.svix.clone()).await?;
    bootstrap::verify_svix_setup(
        &config.svix,
        &config.rest_api_external_url,
        svix_wiring.svix.as_ref(),
        redis_available,
    )
    .await;
    setup_eventbus_handlers(store.clone(), config.clone()).await;

    let ready = Arc::new(AtomicBool::new(true));

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let grpc_server = meteroid::api::server::start_api_server(
        config.clone(),
        store.clone(),
        services.clone(),
        object_store_service.clone(),
        svix_wiring.svix,
        svix_wiring.endpoint_cache,
    );

    let exit = signal::ctrl_c();

    let stripe = Arc::new(StripeClient::new());
    let stripe_adapter = Arc::new(Stripe { client: stripe });

    let rest_server = meteroid::api_rest::server::start_rest_server(
        config.clone(),
        object_store_service.clone(),
        stripe_adapter.clone(),
        store.clone(),
        services.clone(),
        ready.clone(),
        svix_wiring.op_webhook_state,
    );

    tokio::select! {
        grpc_result = grpc_server => {
            if let Err(e) = grpc_result {
                tracing::error!("Error starting gRPC API server: {e:?}");
            }
        },
        rest_result = rest_server => {
            if let Err(e) = rest_result {
                tracing::error!("Error starting REST API server: {e:?}");
            }
        },
        _ = exit => {
              tracing::info!("Interrupted");
        }
    }

    Ok(())
}
