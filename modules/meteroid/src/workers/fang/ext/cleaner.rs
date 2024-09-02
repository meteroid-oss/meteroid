use std::time::Duration;

use crate::workers::fang::ext::config::FangCleanerConfig;
use crate::workers::fang::ext::error::FangExtError;
use crate::workers::fang::ext::metrics;
use diesel::{sql_query, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::{Result, ResultExt};
use meteroid_store::store::PgPool;
use tokio::task::JoinHandle;
use tokio::time::sleep;

#[tracing::instrument(skip(pool))]
pub fn start_cleaner(pool: PgPool, config: FangCleanerConfig) -> JoinHandle<()> {
    log::info!("Starting fang cleaner");

    let sleep_on_nothing_to_delete: Duration =
        Duration::from_secs(config.sleep_seconds_on_nothing_to_delete as u64);
    let sleep_on_error: Duration = Duration::from_secs(config.sleep_seconds_on_error as u64);

    tokio::spawn(async move {
        loop {
            match do_clean(pool.clone(), config.older_than_hours, config.rows_to_delete).await {
                Ok(actually_deleted_rows) if actually_deleted_rows < config.rows_to_delete => {
                    // it doesn't make sense to make DB call again
                    // because we deleted less rows than planned
                    // means there's a high probability that there's nothing left
                    // so chilling with hope that next time we will get something
                    log::info!(
                        "Nothing to remove (sleeping {:?})",
                        sleep_on_nothing_to_delete
                    );
                    sleep(sleep_on_nothing_to_delete).await;
                }
                Ok(actually_deleted_rows) => {
                    log::info!("Successfully removed {} rows", actually_deleted_rows);
                }
                Err(err) => {
                    log::error!(
                        "Failed to run cleaner task (sleeping {:?}): {:?}",
                        sleep_on_error,
                        err
                    );
                    sleep(sleep_on_error).await;
                }
            }
        }
    })
}

#[tracing::instrument(skip_all)]
pub async fn do_clean(
    pool: PgPool,
    older_than_hours: u16,
    limit_delete: u16,
) -> Result<u16, FangExtError> {
    let mut conn = pool
        .get()
        .await
        .change_context(FangExtError::DatabaseConnection)?;

    log::debug!("Running cleaner");

    let deleted_rows: usize = sql_query(
        r#"
                    DELETE FROM "fang_tasks_archive"
                      WHERE id IN (
                        SELECT id
                        FROM "fang_tasks_archive"
                        WHERE archived_at < now() - interval '$1'
                        LIMIT $2
                      )
                "#,
    )
    .bind::<sql_types::VarChar, _>(&format!("{} hours", older_than_hours))
    .bind::<sql_types::Integer, _>(limit_delete as i32)
    .execute(&mut conn)
    .await
    .change_context(FangExtError::DatabaseQuery)
    .attach_printable("Failed to execute prepared statement")?;

    log::debug!("Cleaned {} rows", deleted_rows);

    //todo add node/host/pod as attribute
    metrics::CLEANER_DELETED_ROWS_COUNTER.add(deleted_rows as u64, &[]);

    Ok(deleted_rows as u16)
}
