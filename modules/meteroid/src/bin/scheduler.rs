/*

For production use case, prefer a dedicated scheduler like kubernetes cronjob

*/

use std::sync::Arc;

use common_build_info::BuildInfo;
use common_logging::telemetry;
use meteroid::clients::usage::MeteringUsageClient;
use meteroid::config::Config;
use meteroid::services::currency_rates::OpenexchangeRatesService;
use meteroid::services::invoice_rendering::PdfRenderingService;
use meteroid::services::storage::S3Storage;
use meteroid::singletons;
use meteroid::svix::new_svix;
use meteroid::workers;
use meteroid_mailer::service::mailer_service;
use meteroid_store::Services;
use stripe_client::client::StripeClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {}", build_info);

    let config = Config::get();

    telemetry::init(&config.common.telemetry);

    let store = Arc::new(singletons::get_store().await.clone());
    let svix = new_svix(config);
    let stripe = Arc::new(StripeClient::new());

    let usage_clients = Arc::new(MeteringUsageClient::get().clone());

    let services = Arc::new(Services::new(
        store.clone(),
        usage_clients.clone(),
        svix,
        stripe,
    ));

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
    )?);

    let mailer_service = mailer_service(config.mailer.clone());

    workers::spawn_workers(
        store.clone(),
        services.clone(),
        object_store_service.clone(),
        usage_clients.clone(),
        Arc::new(currency_rate_service),
        pdf_service.clone(),
        mailer_service,
        config,
    )
    .await;

    Ok(())
}
