#[cfg(feature = "metering-server")]
use envconfig::Envconfig;
use std::error::Error;
use std::sync::Arc;
use tokio::signal;

use common_build_info::BuildInfo;
use common_logging::telemetry;
use meteroid::adapters::stripe::Stripe;
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::config::Config;
use meteroid::eventbus::setup_eventbus_handlers;
use meteroid::migrations;
use meteroid::services::currency_rates::OpenexchangeRatesService;
use meteroid::services::invoice_rendering::PdfRenderingService;
use meteroid::services::storage::S3Storage;
use meteroid::svix::new_svix;
use meteroid::workers;
use meteroid::{bootstrap, singletons};
use meteroid_mailer::service::mailer_service;
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

    telemetry::init(&config.common.telemetry);

    let store = singletons::get_store().await;
    let store_arc = Arc::new(store.clone()); // TODO harmonize, arc everywhere or nowhere
    let svix = new_svix(config);
    let stripe = Arc::new(StripeClient::new());

    let usage_clients = Arc::new(MeteringUsageClient::get().clone());

    let services = Services::new(store_arc.clone(), usage_clients.clone(), svix, stripe);

    let services_arc = Arc::new(services.clone());

    migrations::run(&store.pool).await?;
    bootstrap::bootstrap_once(store.clone(), services.clone()).await?;
    setup_eventbus_handlers(store.clone(), config.clone()).await;

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let grpc_server = meteroid::api::server::start_api_server(
        config.clone(),
        store.clone(),
        services.clone(),
        object_store_service.clone(),
    );

    #[cfg(feature = "metering-server")]
    let metering_grpc_server =
        metering::server::start_server(metering::config::Config::init_from_env()?);

    #[cfg(not(feature = "metering-server"))]
    let metering_grpc_server = async {
        log::info!("Metering server is not enabled");
        // sleep forever
        tokio::time::sleep(std::time::Duration::MAX).await;
        Ok::<(), Box<dyn Error>>(())
    };

    let stripe = Arc::new(StripeClient::new());
    let stripe_adapter = Arc::new(Stripe { client: stripe });

    let rest_server = meteroid::api_rest::server::start_rest_server(
        config.clone(),
        object_store_service.clone(),
        stripe_adapter.clone(),
        store.clone(),
        services.clone(),
    );

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let currency_rate_service = OpenexchangeRatesService::new(
        reqwest::Client::new(),
        config.openexchangerates_api_key.clone(),
    );

    let pdf_service = Arc::new(PdfRenderingService::try_new(
        object_store_service.clone(),
        store_arc.clone(),
    )?);

    let mailer_service = mailer_service(config.mailer.clone());

    let workers_handle = tokio::spawn(async move {
        workers::spawn_workers(
            store_arc.clone(),
            services_arc.clone(),
            object_store_service.clone(),
            usage_clients.clone(),
            Arc::new(currency_rate_service),
            pdf_service,
            mailer_service,
            config,
        )
        .await;
    });

    // Wait for shutdown signal or server error or processor error
    tokio::select! {
        metering_grpc_result = metering_grpc_server => {
            if let Err(e) = metering_grpc_result {
                log::error!("Error starting gRPC Metering server: {}", e);
            }
        },
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
        workers_result = workers_handle => {
            match workers_result {
                Ok(_) => log::info!("Workers exited normally"),
                Err(e) => log::error!("Workers error: {}", e),
            }
        },

        _ = signal::ctrl_c() => {
            log::info!("Interrupted");
        }
    }

    Ok(())

    //   tokio::time::sleep(Duration::MAX).await;
}
