use crate::services::currency_rates::CurrencyRatesService;
use crate::services::invoice_rendering::PdfRenderingService;
use crate::services::storage::S3Storage;
use crate::workers;
use crate::workers::pgmq::processors;
use hubspot_client::client::HubspotClient;
use meteroid_store::Services;
use meteroid_store::clients::usage::UsageClient;
use pennylane_client::client::PennylaneClient;
use std::sync::Arc;
use meteroid_mailer::service::MailerService;
use crate::config::Config;

pub mod billing;
pub mod clients;
pub mod fang;
mod metrics;
pub mod misc;
pub mod pgmq;
pub mod webhook_out;

//
// #[derive(Debug, Clone, Envconfig)]
// struct WorkerConfig {
//     #[envconfig(from = "ENABLE_OUTBOX_WORKER", default = "true")]
//     enable_outbox_worker: bool,
//
//     #[envconfig(from = "ENABLE_PDF_WORKER", default = "true")]
//     enable_pdf_worker: bool,
//
//     #[envconfig(from = "ENABLE_WEBHOOK_WORKER", default = "true")]
//     enable_webhook_worker: bool,
//
//     #[envconfig(from = "ENABLE_LIFECYCLE_WORKER", default = "true")]
//     enable_lifecycle_worker: bool,
//
//     #[envconfig(from = "ENABLE_SCHEDULED_WORKER", default = "true")]
//     enable_scheduled_worker: bool,
//
//     #[envconfig(from = "WORKER_CONCURRENCY", default = "1")]
//     worker_concurrency: usize,
// }

pub async fn spawn_workers(
    store: Arc<meteroid_store::Store>,
    services: Arc<Services>,
    object_store_service: Arc<S3Storage>,
    usage_clients: Arc<dyn UsageClient>,
    currency_rates_service: Arc<dyn CurrencyRatesService>,
    pdf_rendering_service: Arc<PdfRenderingService>,
    mailer_service: Arc<dyn MailerService>,
    config: &Config
) {
    let object_store_service1 = object_store_service.clone();
    let object_store_service2 = object_store_service.clone();

    let hubspot_client = Arc::new(HubspotClient::default());
    let pennylane_client = Arc::new(PennylaneClient::default());

    let services_arc1 = services.clone();
    let services_arc2 = services.clone();
    let services_arc3 = services.clone();
    let services_arc4 = services.clone();

    let store_curr = store.clone();
    let store_pgmq1 = store.clone();
    let store_pgmq2 = store.clone();
    let store_pgmq3 = store.clone();
    let store_pgmq4 = store.clone();
    let store_pgmq5 = store.clone();
    let store_pgmq6 = store.clone();
    let store_pgmq7 = store.clone();
    let store_pgmq8 = store.clone();

    // TODO add config to only spawn some
    let mut join_set = tokio::task::JoinSet::new();

    let public_url = config.public_url.clone();
    let rest_api_external_url = config.rest_api_external_url.clone();
    let jwt_secret = config.jwt_secret.clone();

    join_set.spawn(async move {
        processors::run_outbox_dispatch(store_pgmq1).await;
    });

    join_set.spawn(async move {
        processors::run_pdf_render(store_pgmq2, pdf_rendering_service).await;
    });
    join_set.spawn(async move {
        processors::run_webhook_out(store_pgmq3, services_arc3).await;
    });
    join_set.spawn(async move {
        processors::run_metric_sync(store_pgmq4, usage_clients).await;
    });
    join_set.spawn(async move {
        processors::run_hubspot_sync(store_pgmq5, hubspot_client).await;
    });
    join_set.spawn(async move {
        processors::run_pennylane_sync(store_pgmq6, pennylane_client, object_store_service1).await;
    });
    join_set.spawn(async move {
        processors::run_invoice_orchestration(store_pgmq7, services_arc4).await;
    });
    join_set.spawn(async move {
        processors::run_email_sender(store_pgmq8, mailer_service, object_store_service2, public_url, rest_api_external_url, jwt_secret).await;
    });

    join_set.spawn(async move {
        workers::misc::currency_rates_worker::run_currency_rates_worker(
            &store_curr,
            &currency_rates_service,
        )
        .await;
    });
    join_set.spawn(async move {
        workers::billing::lifecycle::run_worker(services_arc1).await;
    });
    join_set.spawn(async move {
        workers::billing::scheduled::run_worker(services_arc2).await;
    });



    join_set.join_all().await;
}
