#[cfg(feature = "metering-server")]
use envconfig::Envconfig;
use std::error::Error;
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
use meteroid::services::credit_note_rendering::CreditNotePdfRenderingService;
use meteroid::services::currency_rates::OpenexchangeRatesService;
use meteroid::services::idempotency::{
    IdempotencyService, InMemoryIdempotencyService, RedisIdempotencyService,
};
use meteroid::services::invoice_rendering::PdfRenderingService;
use meteroid::services::storage::S3Storage;
use meteroid::singletons::connect_redis;
use meteroid::svix::wire_svix;
use meteroid::workers;
use meteroid::{bootstrap, singletons};
use meteroid_mailer::service::mailer_service;
use meteroid_store::Services;
use stripe_client::client::StripeClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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

    let store_arc = Arc::new(store.clone()); // TODO harmonize, arc everywhere or nowhere
    let stripe = Arc::new(StripeClient::new());

    let usage_clients = Arc::new(MeteringUsageClient::get().clone());

    let services = Services::new(store_arc.clone(), usage_clients.clone(), stripe);

    let services_arc = Arc::new(services.clone());

    let fred_client = connect_redis(&config.redis);
    let redis_available = fred_client.is_some();

    let idempotency: Arc<dyn IdempotencyService> = match &fred_client {
        Some(client) => Arc::new(RedisIdempotencyService::new(client.clone())),
        None => Arc::new(InMemoryIdempotencyService::new()),
    };

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
        svix_wiring.svix.clone(),
        svix_wiring.endpoint_cache.clone(),
    );

    #[cfg(feature = "metering-server")]
    let metering_grpc_server = async {
        metering::server::start_server(metering::config::Config::init_from_env()?).await;
        Ok::<(), Box<dyn Error>>(())
    };

    #[cfg(not(feature = "metering-server"))]
    let metering_grpc_server = async {
        tracing::info!("Metering server is not enabled");
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
        ready.clone(),
        svix_wiring.op_webhook_state,
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
        config.public_url.clone(),
        config.jwt_secret.clone(),
    )?);

    let credit_note_pdf_service = Arc::new(CreditNotePdfRenderingService::try_new(
        object_store_service.clone(),
        store_arc.clone(),
    )?);

    let mailer_service = mailer_service(config.mailer.clone());

    let svix_for_workers = svix_wiring.svix.clone();
    let endpoint_cache_for_workers = svix_wiring.endpoint_cache.clone();
    let workers_handle = tokio::spawn(async move {
        workers::spawn_workers(
            store_arc.clone(),
            services_arc.clone(),
            svix_for_workers,
            object_store_service.clone(),
            Arc::new(currency_rate_service),
            pdf_service,
            credit_note_pdf_service,
            mailer_service,
            idempotency,
            endpoint_cache_for_workers,
            config,
        )
        .await;
    });

    // Wait for a shutdown signal or server error or processor error
    tokio::select! {
        metering_grpc_result = metering_grpc_server => {
            if let Err(e) = metering_grpc_result {
                tracing::error!("Error starting gRPC Metering server: {e}");
            }
        },
        grpc_result = grpc_server => {
            if let Err(e) = grpc_result {
                tracing::error!("Error starting gRPC API server: {e}");
            }
        },
        rest_result = rest_server => {
            if let Err(e) = rest_result {
                tracing::error!("Error starting REST API server: {e}");
            }
        },
        workers_result = workers_handle => {
            match workers_result {
                Ok(()) => tracing::info!("Workers exited normally"),
                Err(e) => tracing::error!("Workers error: {e}"),
            }
        },

        _ = signal::ctrl_c() => {
            tracing::info!("Interrupted");
        }
    }

    Ok(())
}
