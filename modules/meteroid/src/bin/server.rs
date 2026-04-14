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
use meteroid::services::svix_cache::{
    NoopSvixEndpointCache, RedisSvixEndpointCache, SvixEndpointCache,
};
use meteroid::singletons::connect_redis;
use meteroid::svix::new_svix;
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

    let endpoint_cache: Arc<dyn SvixEndpointCache> = match &fred_client {
        Some(client) => Arc::new(RedisSvixEndpointCache::new(
            client.clone(),
            config.svix.operational_webhook_secret.is_some(),
        )),
        None => Arc::new(NoopSvixEndpointCache),
    };

    let svix_rate_limiter = Arc::new(meteroid::svix::SvixRateLimiter::new(
        fred_client,
        config.svix.rps_quota,
    ));
    let svix_raw = new_svix(&config.svix);
    let svix: Option<Arc<dyn meteroid::svix::SvixOps>> = svix_raw.map(|s| {
        Arc::new(meteroid::svix::SvixClient::new(s, svix_rate_limiter))
            as Arc<dyn meteroid::svix::SvixOps>
    });

    migrations::run(&store.pool).await?;
    bootstrap::bootstrap_once(store.clone(), svix.clone()).await?;
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
        svix,
    );

    let exit = signal::ctrl_c();

    let stripe = Arc::new(StripeClient::new());
    let stripe_adapter = Arc::new(Stripe { client: stripe });

    let svix_op_state = config
        .svix
        .operational_webhook_secret
        .as_ref()
        .map(|secret| {
            let verifier = svix::webhooks::Webhook::new(secret)
                .expect("Invalid SVIX_OPERATIONAL_WEBHOOK_SECRET");
            meteroid::api_rest::svix_operational::SvixOperationalState {
                webhook_verifier: Arc::new(verifier),
                endpoint_cache: endpoint_cache.clone(),
            }
        });

    let rest_server = meteroid::api_rest::server::start_rest_server(
        config.clone(),
        object_store_service.clone(),
        stripe_adapter.clone(),
        store.clone(),
        services.clone(),
        ready.clone(),
        svix_op_state,
    );

    tokio::select! {
        grpc_result = grpc_server => {
            if let Err(e) = grpc_result {
                log::error!("Error starting gRPC API server: {e:?}");
            }
        },
        rest_result = rest_server => {
            if let Err(e) = rest_result {
                log::error!("Error starting REST API server: {e:?}");
            }
        },
        _ = exit => {
              log::info!("Interrupted");
        }
    }

    Ok(())
}
