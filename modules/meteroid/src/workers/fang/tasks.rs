use std::ops::Deref;

use deadpool_postgres::tokio_postgres::{
    NoTls, Socket,
    tls::{MakeTlsConnect, TlsConnect},
};
use distributed_lock::locks::{DistributedLock, LockKey, PostgresLock};
use fang::{AsyncQueue, AsyncQueueable, AsyncRunnable, AsyncWorkerPool};

use crate::config::Config;

use futures::future::try_join_all;
use meteroid_store::store::{PgPool, get_tls};

pub async fn schedule(
    tasks: Vec<(Box<dyn AsyncRunnable>, LockKey)>,
    config: &Config,
    conn_pool: &PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    if tasks.is_empty() {
        log::warn!("No tasks to schedule");
        return Ok(());
    }

    if let Some(tls) = get_tls(&config.database_url) {
        schedule_tasks_inner(tasks, config, conn_pool, tls).await
    } else {
        schedule_tasks_inner(tasks, config, conn_pool, NoTls).await
    }
}

async fn schedule_tasks_inner<Tls>(
    tasks: Vec<(Box<dyn AsyncRunnable>, LockKey)>,
    config: &Config,
    conn_pool: &PgPool,
    tls: Tls,
) -> Result<(), Box<dyn std::error::Error>>
where
    Tls: MakeTlsConnect<Socket> + Clone + Send + Sync + 'static,
    <Tls as MakeTlsConnect<Socket>>::Stream: Send + Sync,
    <Tls as MakeTlsConnect<Socket>>::TlsConnect: Send,
    <<Tls as MakeTlsConnect<Socket>>::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    let mut queue = AsyncQueue::builder()
        .uri(config.database_url.clone())
        .max_pool_size(5_u32)
        .build();

    queue.connect(tls).await?;
    log::info!("Fang queue connected");

    let futures = tasks
        .iter()
        .map(|task| lock_schedule_task(conn_pool, queue.clone(), task.0.deref(), task.1));

    try_join_all(futures).await?;
    log::info!("Fang tasks scheduled");

    let mut pool = AsyncWorkerPool::<AsyncQueue<Tls>>::builder()
        .number_of_workers(5_u32)
        .queue(queue.clone())
        .retention_mode(fang::RetentionMode::KeepAll)
        .build();

    pool.start().await;
    log::info!("Fang worker pool started");

    Ok(())
}

// see https://github.com/ayrat555/fang/issues/146
// todo it still can produce duplicates due https://github.com/ayrat555/fang/issues/146#issuecomment-1817895187
async fn lock_schedule_task<Tls>(
    pool: &PgPool,
    queue: AsyncQueue<Tls>,
    task: &dyn AsyncRunnable,
    lock_key: LockKey,
) -> Result<(), Box<dyn std::error::Error>>
where
    Tls: MakeTlsConnect<Socket> + Clone + Send + Sync + 'static,
    <Tls as MakeTlsConnect<Socket>>::Stream: Send + Sync,
    <Tls as MakeTlsConnect<Socket>>::TlsConnect: Send,
    <<Tls as MakeTlsConnect<Socket>>::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    let mut client = pool.get().await?;

    let mut lock = PostgresLock::new(&mut client, lock_key);

    if lock.acquire().await? {
        let mut queue = queue;
        queue.schedule_task(task).await?;
        log::info!("Lock {} acquired! Fang task scheduled", lock_key.get());
        lock.release().await?;
    } else {
        log::warn!(
            "Lock {} not acquired! Another instance could run in parallel",
            lock_key.clone().get()
        );
    }

    Ok(())
}
