/*

For production use case, prefer a dedicated scheduler like kubernetes cronjob

*/

use std::sync::Arc;
use std::time::Duration;

use common_build_info::BuildInfo;
use common_logging::init::init_telemetry;
use meteroid::config::Config;
use meteroid::services::invoice_rendering::PdfRenderingService;
use meteroid::services::storage::S3Storage;
use meteroid::singletons;
use meteroid::workers::fang as mfang;
use meteroid::workers::kafka::processors;

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

    let pdf_service = PdfRenderingService::try_new(
        config.gotenberg_url.clone(),
        object_store_service,
        store.clone(),
    )?;

    tokio::try_join!(
        tokio::spawn(async move {
            processors::run_pdf_renderer_outbox_processor(&config.kafka, pdf_service).await;
        }),
        tokio::spawn(async move {
            processors::run_webhook_outbox_processor(&config.kafka, store.clone()).await;
        }),
        // ...
    )?;

    tokio::time::sleep(Duration::MAX).await;

    Ok(())
}
