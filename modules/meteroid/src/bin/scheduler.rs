/*

For production use case, prefer a dedicated scheduler like kubernetes cronjob

*/

use std::sync::Arc;
use std::time::Duration;

use common_build_info::BuildInfo;
use common_logging::init::init_telemetry;
use hubspot_client::client::HubspotClient;
use meteroid::config::Config;
use meteroid::services::invoice_rendering::PdfRenderingService;
use meteroid::services::storage::S3Storage;
use meteroid::singletons;
use meteroid::workers::{fang as mfang, pgmq};
use pennylane_client::client::PennylaneClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {}", build_info);

    let config = Config::get();
    let pool = &singletons::get_store().await.pool;

    init_telemetry(&config.common.telemetry, env!("CARGO_BIN_NAME"));

    // kicking background jobs
    mfang::ext::start_tasks(pool.clone(), &config.fang_ext);

    mfang::tasks::schedule(
        vec![
            // (Box::new(DraftWorker), LockKey::InvoicingDraft),
            // (
            //     Box::new(PendingStatusWorker),
            //     LockKey::InvoicingPendingStatus,
            // ),
            // (Box::new(PriceWorker), LockKey::InvoicingPrice),
            // (Box::new(FinalizeWorker), LockKey::InvoicingFinalize),
            // (Box::new(IssueWorker), LockKey::InvoicingIssue),
            // (Box::new(CurrencyRatesWorker), LockKey::CurrencyRates),
        ],
        config,
        pool,
    )
    .await?;

    let store = Arc::new(singletons::get_store().await.clone());

    let object_store_service = Arc::new(S3Storage::try_new(
        &config.object_store_uri,
        &config.object_store_prefix,
    )?);

    let object_store_service1 = object_store_service.clone();

    let pdf_service = Arc::new(PdfRenderingService::try_new(
        object_store_service,
        store.clone(),
    )?);

    let hubspot_client = Arc::new(HubspotClient::default());
    let pennylane_client = Arc::new(PennylaneClient::default());

    let store_pgmq1 = store.clone();
    let store_pgmq2 = store.clone();
    let store_pgmq3 = store.clone();
    let store_pgmq4 = store.clone();
    let store_pgmq5 = store.clone();

    tokio::try_join!(
        // to run several processors concurrently, just copy the tokio::spawn
        tokio::spawn(async move {
            pgmq::processors::run_outbox_dispatch(store_pgmq1).await;
        }),
        tokio::spawn(async move {
            pgmq::processors::run_pdf_render(store_pgmq2, pdf_service).await;
        }),
        tokio::spawn(async move {
            pgmq::processors::run_webhook_out(store_pgmq3).await;
        }),
        tokio::spawn(async move {
            pgmq::processors::run_hubspot_sync(store_pgmq4, hubspot_client).await;
        }),
        tokio::spawn(async move {
            pgmq::processors::run_pennylane_sync(
                store_pgmq5,
                pennylane_client,
                object_store_service1,
            )
            .await;
        }),
        // tokio::spawn(async move {
        //     processors::run_pdf_renderer_outbox_processor(&config.kafka, pdf_service).await;
        // }),
        // tokio::spawn(async move {
        //     processors::run_webhook_outbox_processor(&config.kafka, store.clone()).await;
        // }),
        // ...
    )?;

    tokio::time::sleep(Duration::MAX).await;

    Ok(())
}
