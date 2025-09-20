use crate::config::Config;
use crate::services::currency_rates::CurrencyRatesService;
use crate::services::invoice_rendering::PdfRenderingService;
use crate::services::storage::S3Storage;
use crate::workers::pgmq::processors;
use hubspot_client::client::HubspotClient;
use meteroid_mailer::service::MailerService;
use meteroid_store::Services;
use meteroid_store::clients::usage::UsageClient;
use pennylane_client::client::PennylaneClient;
use std::sync::Arc;

pub mod billing;
pub mod clients;
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

#[allow(clippy::too_many_arguments)]
pub async fn spawn_workers(
    store: Arc<meteroid_store::Store>,
    services: Arc<Services>,
    object_store_service: Arc<S3Storage>,
    usage_clients: Arc<dyn UsageClient>,
    currency_rates_service: Arc<dyn CurrencyRatesService>,
    pdf_rendering_service: Arc<PdfRenderingService>,
    mailer_service: Arc<dyn MailerService>,
    config: &Config,
) {
    let hubspot_client = Arc::new(HubspotClient::default());
    let pennylane_client = Arc::new(PennylaneClient::default());

    // TODO add config to only spawn some
    let mut join_set = tokio::task::JoinSet::new();

    let public_url = config.public_url.clone();
    let rest_api_external_url = config.rest_api_external_url.clone();
    let jwt_secret = config.jwt_secret.clone();

    {
        let store = store.clone();
        join_set.spawn(async move {
            processors::run_outbox_dispatch(store).await;
        });
    }

    {
        let store = store.clone();
        join_set.spawn(async move {
            processors::run_pdf_render(store, pdf_rendering_service).await;
        });
    }
    {
        let store = store.clone();
        let services = services.clone();
        join_set.spawn(async move {
            processors::run_webhook_out(store, services).await;
        });
    }
    {
        let store = store.clone();
        join_set.spawn(async move {
            processors::run_metric_sync(store, usage_clients).await;
        });
    }
    {
        if config.oauth.hubspot.is_enabled() {
            let store = store.clone();
            join_set.spawn(async move {
                processors::run_hubspot_sync(store, hubspot_client).await;
            });
        } else {
            log::warn!("Hubspot OAuth is not configured, skipping Hubspot sync worker.");
        }
    }
    {
        if config.oauth.pennylane.is_enabled() {
            let store = store.clone();
            let object_store_service = object_store_service.clone();
            join_set.spawn(async move {
                processors::run_pennylane_sync(store, pennylane_client, object_store_service).await;
            });
        } else {
            log::warn!("Pennylane OAuth is not configured, skipping Pennylane sync worker.");
        }
    }
    {
        let store = store.clone();
        let services = services.clone();
        join_set.spawn(async move {
            processors::run_invoice_orchestration(store, services).await;
        });
    }
    {
        let store = store.clone();
        let object_store_service = object_store_service.clone();
        join_set.spawn(async move {
            processors::run_email_sender(
                store,
                mailer_service,
                object_store_service,
                public_url,
                rest_api_external_url,
                jwt_secret,
            )
            .await;
        });
    }

    join_set.spawn(async move {
        misc::currency_rates_worker::run_currency_rates_worker(&store, &currency_rates_service)
            .await;
    });
    {
        let services = services.clone();
        join_set.spawn(async move {
            billing::lifecycle::run_worker(services).await;
        });
    }
    {
        let services = services.clone();
        join_set.spawn(async move {
            billing::scheduled::run_worker(services).await;
        });
    }

    join_set.join_all().await;
}
