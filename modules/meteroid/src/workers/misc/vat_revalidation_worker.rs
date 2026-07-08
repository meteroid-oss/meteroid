use crate::errors;
use error_stack::{Report, ResultExt};
use meteroid_store::Store;
use meteroid_store::domain::pgmq::{PgmqMessageNew, PgmqQueue, VatValidationRequestEvent};
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use std::sync::Arc;
use std::time::Duration;

const RUN_INTERVAL_SECS: u64 = 24 * 3600;
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
pub async fn run_vat_revalidation_worker(store: Arc<Store>) {
    loop {
        // Simple jitter for easy concurrency
        let jitter_duration = Duration::from_secs(rand::random::<u64>() % 3600);

        match enqueue_due_revalidations(&store).await {
            Ok(count) if count > 0 => {
                log::info!("VAT revalidation: enqueued {count} VIES re-checks");
            }
            Ok(_) => {}
            Err(err) => {
                log::error!("VAT revalidation worker encountered error: {err:?}");
            }
        }

        tokio::time::sleep(Duration::from_secs(RUN_INTERVAL_SECS) + jitter_duration).await;
    }
}

async fn enqueue_due_revalidations(
    store: &Arc<Store>,
) -> Result<usize, Report<errors::WorkerError>> {
    let now = chrono::Utc::now().naive_utc();

    let candidates = store
        .list_vat_revalidation_candidates(
            now - chrono::Duration::days(REVALIDATE_AFTER_DAYS),
            now - chrono::Duration::days(MIN_AGE_DAYS),
            BATCH_SIZE,
        )
        .await
        .change_context(errors::WorkerError::VatRevalidation)?;

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
            .await
            .change_context(errors::WorkerError::VatRevalidation)?;
    }

    Ok(count)
}
