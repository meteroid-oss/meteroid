use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::{PgmqResult, sleep_with_jitter};
use common_domain::pgmq::{MessageId, MessageReadQty, MessageReadVtSec, ReadCt};
use error_stack::ResultExt;
use itertools::Itertools;
use meteroid_store::Store;
use meteroid_store::domain::pgmq::{PgmqMessage, PgmqQueue};
use meteroid_store::repositories::pgmq::PgmqInterface;
use std::sync::Arc;
use std::time::Duration;

#[async_trait::async_trait]
pub trait PgmqHandler: Send + Sync {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>>;
}

pub(crate) struct ProcessorConfig {
    pub name: String,
    pub queue: PgmqQueue,
    pub handler: Arc<dyn PgmqHandler>,
    pub store: Arc<Store>,
    pub qty: MessageReadQty,
    pub vt: MessageReadVtSec,
    pub delete_succeeded: bool,
    pub sleep_duration: Duration,
    pub max_read_count: ReadCt,
}

const BACKOFF_DURATION: Duration = Duration::from_secs(1);

pub(crate) async fn run(cfg: ProcessorConfig) {
    log::info!("Starting pgmq dequeuer {}...", cfg.name.as_str());
    loop {
        let perf_start = std::time::Instant::now();
        match run_once(
            cfg.queue,
            cfg.handler.clone(),
            cfg.store.clone(),
            cfg.qty,
            cfg.vt,
            cfg.delete_succeeded,
            cfg.max_read_count,
        )
        .await
        {
            Err(e) => {
                log::warn!(
                    "[{}] Failed to process pgmq {}: {:?}",
                    cfg.name,
                    cfg.queue,
                    e
                );
                sleep_with_jitter(cfg.sleep_duration + BACKOFF_DURATION).await;
            }
            Ok(count) => {
                if count.0 > 0 {
                    let elapsed = perf_start.elapsed();
                    let per_msg = elapsed.as_millis() as f64 / count.0 as f64;
                    log::info!(
                        "[{}] processed {} messages from pgmq {} in {:?} ({:.2} ms/msg)",
                        cfg.name,
                        count.0,
                        cfg.queue,
                        elapsed,
                        per_msg,
                    );
                }
                if count < cfg.qty {
                    log::debug!("[{}] caught up with the queue", cfg.name);
                    // caught up with the queue
                    sleep_with_jitter(cfg.sleep_duration).await;
                }
            }
        }
    }
}

pub(crate) async fn run_once(
    queue: PgmqQueue,
    handler: Arc<dyn PgmqHandler>,
    store: Arc<Store>,
    qty: MessageReadQty,
    vt: MessageReadVtSec,
    delete_processed: bool,
    max_read_count: ReadCt,
) -> PgmqResult<MessageReadQty> {
    let messages = store
        .pgmq_read(queue, qty, vt)
        .await
        .change_context(PgmqError::ReadMessages)?;

    if messages.is_empty() {
        return Ok(MessageReadQty(0));
    }

    let (too_old_messages, messages): (Vec<_>, Vec<_>) = messages
        .into_iter()
        .partition(|x| x.read_ct > max_read_count);

    // todo dlq
    if !too_old_messages.is_empty() {
        let ids = too_old_messages.iter().map(|x| x.msg_id).collect();
        log::warn!("[{queue}] Found too old messages: {ids:?}, archiving...");
        store
            .pgmq_archive(queue, ids)
            .await
            .change_context(PgmqError::ArchiveMessages)?;
    }

    let read_len = too_old_messages.len() + messages.len();

    let success_ids = handler.handle(&messages).await?;

    if delete_processed {
        store
            .pgmq_delete(queue, success_ids)
            .await
            .change_context(PgmqError::DeleteMessages)?;
    } else {
        store
            .pgmq_archive(queue, success_ids)
            .await
            .change_context(PgmqError::ArchiveMessages)?;
    }

    Ok(MessageReadQty(read_len as i16))
}

pub(crate) struct Noop;

#[async_trait::async_trait]
impl PgmqHandler for Noop {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        Ok(msgs.iter().map(|x| x.msg_id).collect_vec())
    }
}
