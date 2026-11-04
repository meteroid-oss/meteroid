use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[cfg(feature = "metering-server")]
use envconfig::Envconfig;
use meteroid_mailer::service::mailer_service;
use meteroid_store::Services;
use tokio::signal;

use crate::eventbus::setup_eventbus_handlers;
use crate::module::Modules;
use crate::process::{self, Process, connect_redis};
use crate::services::credit_note_rendering::CreditNotePdfRenderingService;
use crate::services::currency_rates::OpenexchangeRatesService;
use crate::services::idempotency::{
    IdempotencyService, InMemoryIdempotencyService, RedisIdempotencyService,
};
use crate::services::invoice_rendering::PdfRenderingService;
use crate::services::storage::S3Storage;
use crate::{bootstrap, migrations, workers};

/// The gRPC and REST APIs of an edition.
pub async fn run_api(edition: impl FnOnce() -> Modules) -> Result<(), Box<dyn std::error::Error>> {
    let Process {
        config,
        modules,
        store,
        usage_client,
        egress,
    } = process::start(edition)?;
    let services = Services::new(Arc::new(store.clone()), Arc::new(usage_client));

    // One client (connection pool) shared by every Redis-backed service.
    let fred_client = connect_redis(&config.redis).await?;

    migrations::run(&store.pool).await?;
    // Once the schema is current: edition state (e.g. the instance license) lives in it.
    modules.startup(&store, &config).await;
    bootstrap::bootstrap_once(store.clone(), &config.instance_bootstrap).await?;
    setup_eventbus_handlers(store.clone(), config.clone(), &modules).await;

    let ready = Arc::new(AtomicBool::new(true));

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let rate_limiter = modules.rate_limiter(fred_client.as_ref()).await;

    let grpc_server = crate::api::server::start_api_server(
        config.clone(),
        store.clone(),
        services.clone(),
        object_store_service.clone(),
        rate_limiter.clone(),
        fred_client,
        modules.clone(),
        egress.clone(),
        std::sync::Arc::new(crate::api_rest::webhook_catalog::RestWebhookCatalog),
    );

    let rest_server = crate::api_rest::server::start_rest_server(
        config.clone(),
        object_store_service.clone(),
        store.clone(),
        services.clone(),
        ready.clone(),
        rate_limiter.clone(),
        modules,
        egress,
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
        _ = signal::ctrl_c() => {
              tracing::info!("Interrupted");
        }
    }

    Ok(())
}

pub use meteroid_workers::run::run_workers;

type MeteringServer = Pin<Box<dyn Future<Output = Result<(), Box<dyn std::error::Error>>>>>;

/// API, workers and, with `metering-server`, the metering server in one process.
/// Development and preview environments only.
pub async fn run_standalone(
    edition: impl FnOnce() -> Modules + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "metering-server")]
    let metering = serve_metering(|_| metering_server::server::ExtraRoutes::default());

    #[cfg(not(feature = "metering-server"))]
    let metering: MeteringServer = Box::pin(async {
        tracing::info!("Metering server is not enabled");
        // sleep forever
        tokio::time::sleep(std::time::Duration::MAX).await;
        Ok::<(), Box<dyn std::error::Error>>(())
    });

    run_standalone_serving(edition, metering).await
}

/// [`run_standalone`], with the metering port also serving the routes `extra` builds from the
/// metering config.
#[cfg(feature = "metering-server")]
pub async fn run_standalone_with_metering(
    edition: impl FnOnce() -> Modules + 'static,
    extra: impl FnOnce(&metering::config::Config) -> metering_server::server::ExtraRoutes + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    run_standalone_serving(edition, serve_metering(extra)).await
}

#[cfg(feature = "metering-server")]
fn serve_metering(
    extra: impl FnOnce(&metering::config::Config) -> metering_server::server::ExtraRoutes + 'static,
) -> MeteringServer {
    // Polled after the dotenv is loaded, so the metering config reads it.
    Box::pin(async move {
        let config = metering::config::Config::init_from_env()?;
        let extra = extra(&config);
        metering_server::server::start_server_with(
            config,
            metering_server::server::ServerMode::Combined,
            extra,
        )
        .await
    })
}

async fn run_standalone_serving(
    edition: impl FnOnce() -> Modules,
    metering_grpc_server: MeteringServer,
) -> Result<(), Box<dyn std::error::Error>> {
    let Process {
        config,
        modules,
        store,
        usage_client,
        egress,
    } = process::start(edition)?;

    let store_arc = Arc::new(store.clone());

    let usage_clients = Arc::new(usage_client);

    let services = Services::new(store_arc.clone(), usage_clients.clone());

    let services_arc = Arc::new(services.clone());

    // One client (connection pool) shared by every Redis-backed service.
    let fred_client = connect_redis(&config.redis).await?;

    let _idempotency: Arc<dyn IdempotencyService> = match &fred_client {
        Some(client) => Arc::new(RedisIdempotencyService::new(client.clone())),
        None => Arc::new(InMemoryIdempotencyService::new()),
    };

    migrations::run(&store.pool).await?;
    // Once the schema is current: edition state (e.g. the instance license) lives in it.
    modules.startup(&store, &config).await;
    bootstrap::bootstrap_once(store.clone(), &config.instance_bootstrap).await?;
    setup_eventbus_handlers(store.clone(), config.clone(), &modules).await;

    let ready = Arc::new(AtomicBool::new(true));

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let rate_limiter = modules.rate_limiter(fred_client.as_ref()).await;

    let idempotency: Arc<dyn IdempotencyService> = match &fred_client {
        Some(client) => Arc::new(RedisIdempotencyService::new(client.clone())),
        None => Arc::new(InMemoryIdempotencyService::new()),
    };

    let grpc_server = crate::api::server::start_api_server(
        config.clone(),
        store.clone(),
        services.clone(),
        object_store_service.clone(),
        rate_limiter.clone(),
        fred_client,
        modules.clone(),
        egress.clone(),
        std::sync::Arc::new(crate::api_rest::webhook_catalog::RestWebhookCatalog),
    );

    let rest_server = crate::api_rest::server::start_rest_server(
        config.clone(),
        object_store_service.clone(),
        store.clone(),
        services.clone(),
        ready.clone(),
        rate_limiter.clone(),
        modules.clone(),
        egress.clone(),
    );

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let currency_rate_service = OpenexchangeRatesService::new(
        OpenexchangeRatesService::http_client(),
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

    let workers_handle = tokio::spawn(async move {
        workers::spawn_workers(
            store_arc.clone(),
            services_arc.clone(),
            usage_clients.clone(),
            object_store_service.clone(),
            Arc::new(currency_rate_service),
            pdf_service,
            credit_note_pdf_service,
            mailer_service,
            idempotency,
            &config,
            &modules,
            &egress,
        )
        .await
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
                Ok(Ok(())) => tracing::info!("Workers exited normally"),
                Ok(Err(e)) => tracing::error!("Workers could not start: {e}"),
                Err(e) => tracing::error!("Workers error: {e}"),
            }
        },

        _ = signal::ctrl_c() => {
            tracing::info!("Interrupted");
        }
    }

    Ok(())
}
