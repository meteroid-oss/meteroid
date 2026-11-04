use std::sync::Arc;

use meteroid_kit::eventbus::setup_eventbus_handlers;
use meteroid_kit::process::{self, Process, connect_redis};
use meteroid_mailer::service::mailer_service;
use meteroid_store::Services;

use meteroid_app::migrations;
use meteroid_app::services::credit_note_rendering::CreditNotePdfRenderingService;
use meteroid_app::services::currency_rates::OpenexchangeRatesService;
use meteroid_app::services::idempotency::{
    IdempotencyService, InMemoryIdempotencyService, RedisIdempotencyService,
};
use meteroid_app::services::invoice_rendering::PdfRenderingService;
use meteroid_app::services::storage::S3Storage;
use meteroid_kit::Modules;

/// The workers of an edition (scheduled jobs, queue consumers).
pub async fn run_workers(
    edition: impl FnOnce() -> Modules,
) -> Result<(), Box<dyn std::error::Error>> {
    let Process {
        config,
        modules,
        store,
        usage_client,
        egress,
    } = process::start(edition)?;
    let store = Arc::new(store);
    setup_eventbus_handlers((*store).clone(), config.clone(), &modules).await;
    let usage_clients = Arc::new(usage_client);
    let services = Arc::new(Services::new(store.clone(), usage_clients.clone()));

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
        store.clone(),
        config.public_url.clone(),
        config.jwt_secret.clone(),
    )?);

    let credit_note_pdf_service = Arc::new(CreditNotePdfRenderingService::try_new(
        object_store_service.clone(),
        store.clone(),
    )?);

    let mailer_service = mailer_service(config.mailer.clone());

    let fred_client = connect_redis(&config.redis).await?;

    let idempotency: Arc<dyn IdempotencyService> = match &fred_client {
        Some(client) => Arc::new(RedisIdempotencyService::new(client.clone())),
        None => Arc::new(InMemoryIdempotencyService::new()),
    };

    // Producers publish to topic bindings created by migrations; ensure they exist
    // before any worker runs, else events publish to unbound keys and vanish.
    migrations::run(&store.pool).await?;
    // Once the schema is current: edition state (e.g. the instance license) lives in it.
    modules.startup(&store, &config).await;

    crate::spawn_workers(
        store.clone(),
        services.clone(),
        usage_clients.clone(),
        object_store_service.clone(),
        Arc::new(currency_rate_service),
        pdf_service.clone(),
        credit_note_pdf_service,
        mailer_service,
        idempotency,
        &config,
        &modules,
        &egress,
    )
    .await?;

    Ok(())
}
