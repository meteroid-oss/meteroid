/*

For production use case, prefer a dedicated scheduler like kubernetes cronjob

*/

use std::time::Duration;

use common_build_info::BuildInfo;
use common_logging::init::init_telemetry;
use distributed_lock::locks::LockKey;
use meteroid::config::Config;
use meteroid::singletons::get_pool;
use meteroid::workers::fang;
use meteroid::workers::invoicing::draft_worker::DraftWorker;
use meteroid::workers::invoicing::finalize_worker::FinalizeWorker;
use meteroid::workers::invoicing::issue_worker::IssueWorker;
use meteroid::workers::invoicing::pending_status_worker::PendingStatusWorker;
use meteroid::workers::invoicing::price_worker::PriceWorker;
use meteroid::workers::misc::currency_rates_worker::CurrencyRatesWorker;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {}", build_info);

    let config = Config::get();
    let pool = get_pool();

    init_telemetry(&config.common.telemetry, env!("CARGO_BIN_NAME"));

    // kicking background jobs
    fang::ext::start_tasks(pool.clone(), &config.fang_ext);

    fang::tasks::schedule(
        vec![
            (Box::new(DraftWorker), LockKey::InvoicingDraft),
            (
                Box::new(PendingStatusWorker),
                LockKey::InvoicingPendingStatus,
            ),
            (Box::new(PriceWorker), LockKey::InvoicingPrice),
            (Box::new(FinalizeWorker), LockKey::InvoicingFinalize),
            (Box::new(IssueWorker), LockKey::InvoicingIssue),
            (Box::new(CurrencyRatesWorker), LockKey::CurrencyRates),
        ],
        config,
        pool,
    )
    .await?;

    tokio::time::sleep(Duration::MAX).await;

    Ok(())
}
