use crate::{errors, repo::get_pool};
use deadpool_postgres::Pool;
use fang::{AsyncQueueable, AsyncRunnable, Deserialize, FangError, Scheduled, Serialize};
use meteroid_repository as db;

use crate::workers::metrics::record_call;
use common_utils::timed::TimedExt;
use error_stack::{Result, ResultExt};

#[derive(Serialize, Deserialize)]
#[serde(crate = "fang::serde")]
pub struct PendingStatusWorker;

#[async_trait::async_trait]
#[typetag::serde]
impl AsyncRunnable for PendingStatusWorker {
    #[tracing::instrument(skip_all)]
    async fn run(&self, _queue: &mut dyn AsyncQueueable) -> core::result::Result<(), FangError> {
        let pool = get_pool();

        pending_worker(pool)
            .timed(|res, elapsed| record_call("pending", res, elapsed))
            .await
            .map_err(|err| {
                log::error!("Error in pending_status worker: {}", err);
                FangError {
                    description: err.to_string(),
                }
            })
    }

    fn uniq(&self) -> bool {
        true
    }

    fn cron(&self) -> Option<Scheduled> {
        let expression = "0 1/10 * * * * *"; // every 10 minutes
        Some(Scheduled::CronPattern(expression.to_string()))
    }

    fn max_retries(&self) -> i32 {
        0
    }
}

/**
 * We get all the invoices that are not finalized and not voided, where the end date is passed and grace period is not over
 * and update their status to pending
 */
#[tracing::instrument(skip_all)]
pub async fn pending_worker(pool: &Pool) -> Result<(), errors::WorkerError> {
    let connection = pool
        .get()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    db::invoices::update_pending_finalization_invoices()
        .bind(&connection)
        .all()
        .await
        .change_context(errors::WorkerError::DatabaseError)?;

    Ok(())
}
