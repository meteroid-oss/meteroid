use distributed_lock::LeaderElection;
use meteroid_app::config::ParsedCron;
use meteroid_store::Store;
use meteroid_store::domain::pgmq::{PgmqMessageNew, PgmqQueue, VatValidationRequestEvent};
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use std::sync::Arc;

/// Best-effort daily budget, kept well under VIES rate limits. Oldest-checked
/// customers go first, so a backlog drains across consecutive runs.
const BATCH_SIZE: i64 = 500;
/// Per-customer re-check cadence. VAT registrations do get revoked, and VIES
/// answers change; peers revalidate monthly (Hyperline) to quarterly (Chargebee).
const REVALIDATE_AFTER_DAYS: i64 = 30;
/// Grace period during which the event-driven initial validation owns the row.
const MIN_AGE_DAYS: i64 = 1;

/// Best-effort periodic VIES re-validation. Also backfills customers whose VAT
/// number predates external validation (status still unset). The queue worker
/// treats these `revalidate` jobs as single-shot and never downgrades a
/// definitive VALID/INVALID on VIES unavailability.
///
/// Leader-elected: the candidate scan takes no claim, so every replica would
/// enqueue the same rows and multiply VIES traffic by the replica count.
pub async fn run_vat_revalidation_worker(
    store: Arc<Store>,
    elector: Arc<dyn LeaderElection>,
    schedule: ParsedCron,
) {
    super::run_scheduled("vat revalidation", schedule, elector, move || {
        let store = store.clone();
        async move { run_once(&store).await }
    })
    .await;
}

async fn run_once(store: &Arc<Store>) -> meteroid_store::StoreResult<()> {
    let now = chrono::Utc::now().naive_utc();

    let candidates = store
        .list_vat_revalidation_candidates(
            now - chrono::Duration::days(REVALIDATE_AFTER_DAYS),
            now - chrono::Duration::days(MIN_AGE_DAYS),
            BATCH_SIZE,
        )
        .await?;

    // Format validity is filtered in SQL; VIES coverage (EU-27 + XI) only here.
    let messages: Vec<PgmqMessageNew> = candidates
        .into_iter()
        .filter_map(|customer| {
            let vat_number = customer
                .vat_number
                .filter(|vat| meteroid_tax::vies::is_vies_eligible(vat))?;
            VatValidationRequestEvent {
                tenant_id: customer.tenant_id,
                customer_id: customer.id,
                vat_number,
                attempt: 0,
                revalidate: true,
            }
            .try_into()
            .ok()
        })
        .collect();

    let count = messages.len();
    if count > 0 {
        store
            .pgmq_send_batch(PgmqQueue::VatValidation, messages)
            .await?;
        log::info!("VAT revalidation: enqueued {count} VIES re-checks");
    }

    Ok(())
}
