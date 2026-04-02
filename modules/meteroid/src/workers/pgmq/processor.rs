use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::{PgmqResult, sleep_with_jitter};
use common_domain::pgmq::{MessageId, MessageReadQty, MessageReadVtSec, ReadCt};
use common_logging::GLOBAL_METER;
use error_stack::ResultExt;
use itertools::Itertools;
use meteroid_store::Store;
use meteroid_store::domain::dead_letter::DeadLetterMessageNew;
use meteroid_store::domain::pgmq::extract_tenant_id_from_headers;
use meteroid_store::domain::pgmq::{PgmqMessage, PgmqQueue};
use meteroid_store::repositories::dead_letter::DeadLetterInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

static MESSAGES_PROCESSED: std::sync::LazyLock<Counter<u64>> = std::sync::LazyLock::new(|| {
    GLOBAL_METER
        .u64_counter("pgmq_messages_processed")
        .with_description("Messages processed from PGMQ queues")
        .build()
});

static MESSAGES_DEAD_LETTERED: std::sync::LazyLock<Counter<u64>> = std::sync::LazyLock::new(|| {
    GLOBAL_METER
        .u64_counter("pgmq_messages_dead_lettered")
        .with_description("Messages moved to dead letter queue")
        .build()
});

static PROCESSING_DURATION: std::sync::LazyLock<Histogram<f64>> = std::sync::LazyLock::new(|| {
    GLOBAL_METER
        .f64_histogram("pgmq_processing_duration_ms")
        .with_description("Batch processing duration in milliseconds")
        .with_unit("ms")
        .build()
});

pub struct HandleResult {
    pub succeeded: Vec<MessageId>,
    pub failed: Vec<(MessageId, String)>,
}

impl HandleResult {
    pub fn from_succeeded(ids: Vec<MessageId>) -> Self {
        Self {
            succeeded: ids,
            failed: vec![],
        }
    }

    pub fn fail(id: MessageId, err: &dyn std::fmt::Debug) -> (MessageId, String) {
        (id, format!("{err:?}"))
    }
}

fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[async_trait::async_trait]
pub trait PgmqHandler: Send + Sync {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult>;
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
                log::error!(
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
                    log::debug!(
                        "[{}] processed {} messages from pgmq {} in {:?} ({:.2} ms/msg)",
                        cfg.name,
                        count.0,
                        cfg.queue,
                        elapsed,
                        per_msg,
                    );
                    PROCESSING_DURATION.record(
                        elapsed.as_millis() as f64,
                        &[KeyValue::new("queue", cfg.queue.as_str())],
                    );
                }
                if count < cfg.qty {
                    log::debug!("[{}] caught up with the queue", cfg.name);
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
    let messages = match tokio::time::timeout(
        Duration::from_secs(10),
        store.pgmq_read(queue, qty, vt),
    )
    .await
    {
        Ok(result) => result.change_context(PgmqError::ReadMessages)?,
        Err(_) => {
            log::error!("[{queue}] pgmq_read timed out after 10 seconds");
            return Err(error_stack::Report::new(PgmqError::ReadMessagesTimeout));
        }
    };

    if messages.is_empty() {
        return Ok(MessageReadQty(0));
    }

    let read_len = messages.len();

    // Which messages have exhausted retries? We still run them through the handler
    // so we capture the actual error on this final attempt.
    let exhausted_ids: HashSet<i64> = messages
        .iter()
        .filter(|m| m.read_ct > max_read_count)
        .map(|m| m.msg_id.0)
        .collect();

    // Run handler on ALL messages (including exhausted ones)
    let handle_result = handler.handle(&messages).await?;

    // Build lookup structures
    let succeeded_ids: HashSet<i64> = handle_result.succeeded.iter().map(|id| id.0).collect();
    let failed_errors: HashMap<i64, &str> = handle_result
        .failed
        .iter()
        .map(|(id, err)| (id.0, err.as_str()))
        .collect();

    // Metrics
    let queue_label = KeyValue::new("queue", queue.as_str());
    if !handle_result.succeeded.is_empty() {
        MESSAGES_PROCESSED.add(
            handle_result.succeeded.len() as u64,
            &[queue_label.clone(), KeyValue::new("outcome", "success")],
        );
    }
    if !handle_result.failed.is_empty() {
        MESSAGES_PROCESSED.add(
            handle_result.failed.len() as u64,
            &[queue_label, KeyValue::new("outcome", "failure")],
        );
    }

    // Dead-letter: exhausted messages that did NOT succeed on this final attempt
    let to_dead_letter: Vec<&PgmqMessage> = messages
        .iter()
        .filter(|m| exhausted_ids.contains(&m.msg_id.0) && !succeeded_ids.contains(&m.msg_id.0))
        .collect();

    if !to_dead_letter.is_empty() {
        let dlq_entries: Vec<DeadLetterMessageNew> = to_dead_letter
            .iter()
            .map(|msg| {
                let last_error = failed_errors
                    .get(&msg.msg_id.0)
                    .map(|e| strip_ansi(e))
                    .unwrap_or_else(|| "Handler did not report error details".to_string());

                let headers_json = msg.headers.as_ref().map(|h| h.0.clone());
                let tenant_id = extract_tenant_id_from_headers(&headers_json);

                DeadLetterMessageNew {
                    tenant_id,
                    queue: queue.as_str().to_string(),
                    pgmq_msg_id: msg.msg_id.0,
                    message: msg.message.as_ref().map(|m| m.0.clone()),
                    headers: msg.headers.as_ref().map(|h| h.0.clone()),
                    read_ct: msg.read_ct.0,
                    enqueued_at: msg.enqueued_at,
                    last_error: Some(last_error),
                }
            })
            .collect();

        let dlq_ids: Vec<MessageId> = to_dead_letter.iter().map(|m| m.msg_id).collect();
        let count = dlq_ids.len();

        if let Err(e) = store.insert_dead_letter_batch(dlq_entries).await {
            log::error!("[{queue}] Failed to insert dead letter entries: {:?}", e);
        }

        log::warn!("[{queue}] Dead-lettered {count} message(s): {dlq_ids:?}");

        MESSAGES_DEAD_LETTERED.add(count as u64, &[KeyValue::new("queue", queue.as_str())]);

        store
            .pgmq_delete(queue, dlq_ids)
            .await
            .change_context(PgmqError::DeleteMessages)?;
    }

    // Succeeded messages — delete or archive (excluding dead-lettered ones, already handled)
    let to_ack: Vec<MessageId> = handle_result
        .succeeded
        .into_iter()
        .filter(|id| !exhausted_ids.contains(&id.0) || succeeded_ids.contains(&id.0))
        .collect();

    if !to_ack.is_empty() {
        if delete_processed {
            store
                .pgmq_delete(queue, to_ack)
                .await
                .change_context(PgmqError::DeleteMessages)?;
        } else {
            store
                .pgmq_archive(queue, to_ack)
                .await
                .change_context(PgmqError::ArchiveMessages)?;
        }
    }

    // Messages not in succeeded or failed: left in queue, will become visible after VT expires

    Ok(MessageReadQty(read_len as i16))
}

pub(crate) struct Noop;

#[async_trait::async_trait]
impl PgmqHandler for Noop {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        Ok(HandleResult::from_succeeded(
            msgs.iter().map(|x| x.msg_id).collect_vec(),
        ))
    }
}
