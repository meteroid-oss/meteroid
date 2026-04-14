/*

For production use case, prefer a dedicated scheduler like kubernetes cronjob

*/

use std::sync::Arc;

use common_build_info::BuildInfo;
use common_logging::telemetry;
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::config::Config;
use meteroid::services::credit_note_rendering::CreditNotePdfRenderingService;
use meteroid::services::currency_rates::OpenexchangeRatesService;
use meteroid::services::idempotency::{
    IdempotencyService, InMemoryIdempotencyService, RedisIdempotencyService,
};
use meteroid::services::svix_cache::{
    NoopSvixEndpointCache, RedisSvixEndpointCache, SvixEndpointCache,
};
use meteroid::services::invoice_rendering::PdfRenderingService;
use meteroid::services::storage::S3Storage;
use meteroid::singletons;
use meteroid::singletons::connect_redis;
use meteroid::svix::new_svix;
use meteroid::workers;
use meteroid_mailer::service::mailer_service;
use meteroid_store::Services;
use stripe_client::client::StripeClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {build_info}");

    let config = Config::get();

    telemetry::init(&config.common.telemetry);

    let store = Arc::new(singletons::get_store().await.clone());

    let stripe = Arc::new(StripeClient::new());

    let usage_clients = Arc::new(MeteringUsageClient::get().clone());

    let services = Arc::new(Services::new(store.clone(), usage_clients.clone(), stripe));

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
        store.clone(),
        config.public_url.clone(),
        config.jwt_secret.clone(),
    )?);

    let credit_note_pdf_service = Arc::new(CreditNotePdfRenderingService::try_new(
        object_store_service.clone(),
        store.clone(),
    )?);

    let mailer_service = mailer_service(config.mailer.clone());

    let fred_client = connect_redis(&config.redis);
    let (idempotency, endpoint_cache): (
        Arc<dyn IdempotencyService>,
        Arc<dyn SvixEndpointCache>,
    ) = match &fred_client {
        Some(client) => (
            Arc::new(RedisIdempotencyService::new(client.clone())),
            Arc::new(RedisSvixEndpointCache::new(
                client.clone(),
                config.svix.operational_webhook_secret.is_some(),
            )),
        ),
        None => (
            Arc::new(InMemoryIdempotencyService::new()),
            Arc::new(NoopSvixEndpointCache),
        ),
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

    workers::spawn_workers(
        store.clone(),
        services.clone(),
        svix,
        object_store_service.clone(),
        Arc::new(currency_rate_service),
        pdf_service.clone(),
        credit_note_pdf_service,
        mailer_service,
        idempotency,
        endpoint_cache,
        config,
    )
    .await;

    Ok(())
}
