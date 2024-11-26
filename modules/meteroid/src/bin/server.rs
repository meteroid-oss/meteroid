use secrecy::ExposeSecret;
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
use meteroid::migrations;
use meteroid::services::storage::S3Storage;
use meteroid_store::repositories::webhooks::WebhooksInterface;
use meteroid_store::store::StoreConfig;

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

    let metering_channel = tonic::transport::Channel::from_shared(config.metering_endpoint.clone())
        .expect("Invalid metering_endpoint")
        .connect_lazy();
    let metering_layered_channel =
        build_layered_client_service(metering_channel, &config.internal_auth);

    let query_service_client = UsageQueryServiceClient::new(metering_layered_channel.clone());
    let metering_service = MetersServiceClient::new(metering_layered_channel);

    let svix = Arc::new(svix::api::Svix::new(
        config.svix_jwt_token.expose_secret().clone(),
        Some(svix::api::SvixOptions {
            debug: true,
            server_url: Some(config.svix_server_url.clone()),
            timeout: Some(std::time::Duration::from_secs(30)),
        }),
    ));

    let store = meteroid_store::Store::new(StoreConfig {
        database_url: config.database_url.clone(),
        crypt_key: config.secrets_crypt_key.clone(),
        jwt_secret: config.jwt_secret.clone(),
        multi_organization_enabled: config.multi_organization_enabled,
        eventbus: create_eventbus_memory(),
        usage_client: Arc::new(MeteringUsageClient::new(
            query_service_client,
            metering_service,
        )),
        svix: Some(svix.clone()),
    })?;
    // todo this is a hack to register the event types in svix, should be managed by an api
    store.insert_webhook_out_event_types().await?;

    setup_eventbus_handlers(store.clone(), config.clone()).await;

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let grpc_server = meteroid::api::server::start_api_server(
        config.clone(),
        store.clone(),
        object_store_service.clone(),
    );

    let exit = signal::ctrl_c();

    migrations::run(&store.pool).await?;

    let stripe_adapter = Arc::new(Stripe {
        client: stripe_client::client::StripeClient::new(),
    });

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
