use std::time::Duration;

use crate::workers::fang::ext::config::FangArchiverConfig;
use crate::workers::fang::ext::error::FangExtError;
use crate::workers::fang::ext::metrics;
use diesel::{sql_query, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::{Result, ResultExt};
use meteroid_store::store::PgPool;
use tokio::task::JoinHandle;
use tokio::time::sleep;

#[tracing::instrument(skip(pool))]
pub fn start_archiver(pool: PgPool, config: FangArchiverConfig) -> JoinHandle<()> {
    log::info!("Starting fang archiver");

    let sleep_on_nothing_to_move =
        Duration::from_secs(config.sleep_seconds_on_nothing_to_move as u64);

    let sleep_on_error = Duration::from_secs(config.sleep_seconds_on_error as u64);

    tokio::spawn(async move {
        loop {
            match do_archive(pool.clone(), config.older_than_hours, config.rows_to_move).await {
                Ok(actually_moved_rows) if actually_moved_rows < config.rows_to_move => {
                    // it doesn't make sense to make DB call again
                    // because we moved less rows than planned
                    // means there's a high probability that there's nothing left
                    // so chilling with hope that next time we will get something
                    log::info!("Nothing to move (sleeping {:?})", sleep_on_nothing_to_move);
                    sleep(sleep_on_nothing_to_move).await;
                }
                Ok(actually_moved_rows) => {
                    log::info!("Successfully moved {} rows", actually_moved_rows);
                }
                Err(err) => {
                    log::error!(
                        "Failed to run archiver task (sleeping {:?}): {:?}",
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
async fn do_archive(
    pool: PgPool,
    older_than_hours: u16,
    limit_move: u16,
) -> Result<u16, FangExtError> {
    let mut conn = pool
        .get()
        .await
        .change_context(FangExtError::DatabaseConnection)?;

    log::debug!("Running archiving");

    let moved_rows: usize = sql_query(
        r#"
                WITH moved_rows AS (
                    DELETE FROM "fang_tasks" orig
                      WHERE id IN (
                        SELECT id
                        FROM "fang_tasks"
                        WHERE state = 'finished' AND created_at < now() - interval '$1'
                        LIMIT $2
                      )
                      RETURNING orig.*
                  )
                  INSERT INTO "fang_tasks_archive"
                  SELECT *, now() FROM moved_rows
                "#,
    )
    .bind::<sql_types::VarChar, _>(&format!("{} hours", older_than_hours))
    .bind::<sql_types::Integer, _>(limit_move as i32)
    .execute(&mut conn)
    .await
    .change_context(FangExtError::DatabaseQuery)
    .attach_printable("Failed to execute prepared statement")?;

    log::debug!("Archived {} rows", moved_rows);

    //todo add node/host/pod as attribute
    metrics::ARCHIVER_MOVED_ROWS_COUNTER.add(moved_rows as u64, &[]);

    Ok(moved_rows as u16)
}
