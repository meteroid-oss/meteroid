use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use common_domain::pgmq::{MessageId, MessageReadQty, MessageReadVtSec, ReadCt};
use error_stack::ResultExt;
use meteroid_store::Store;
use meteroid_store::domain::pgmq::{PgmqMessage, PgmqQueue};
use meteroid_store::repositories::pgmq::PgmqInterface;
use rand::Rng;
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

pub(crate) async fn run(cfg: ProcessorConfig) {
    log::info!("Starting pgmq dequeuer {}...", cfg.name.as_str());
    loop {
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
                sleep(cfg.sleep_duration, 100).await
            }
            Ok(count) => {
                if count < cfg.qty {
                    log::debug!("[{}] caught up with the queue", cfg.name);
                    // caught up with the queue
                    sleep(cfg.sleep_duration, 100).await
                }
            }
        }
    }
}

async fn run_once(
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
        log::warn!(
            "[{}] Found too old messages: {:?}, archiving...",
            queue,
            ids
        );
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

async fn sleep(duration: Duration, jitter_millis: u64) {
    let jitter = rand::rng().random_range(0..=jitter_millis);
    let total_duration = duration + Duration::from_millis(jitter);

    tokio::time::sleep(total_duration).await;
}
