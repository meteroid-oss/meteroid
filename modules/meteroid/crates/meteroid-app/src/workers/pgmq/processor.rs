use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::utils::sleep_with_jitter;
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
use rand::RngExt;
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

pub struct ProcessorConfig {
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
/// Idle polling backs off up to this interval (or the configured one, if longer).
const MAX_IDLE_SLEEP: Duration = Duration::from_secs(5);
/// First retry delay of a failed message, doubled per failed read. Independent of the read VT,
/// which is sized for processing time: a webhook waiting on provider state must retry promptly.
const RETRY_BASE_DELAY_SECS: i32 = 15;
/// Cap on the exponential retry delay of a failed message.
const MAX_RETRY_DELAY_SECS: i32 = 3600;

pub async fn run(cfg: ProcessorConfig) {
    log::info!("Starting pgmq dequeuer {}...", cfg.name.as_str());

    let max_idle_sleep = cfg.sleep_duration.max(MAX_IDLE_SLEEP);
    let mut idle_sleep = cfg.sleep_duration;

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
                    idle_sleep = cfg.sleep_duration;
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
                    sleep_with_jitter(idle_sleep).await;
                    if count.0 == 0 {
                        idle_sleep = (idle_sleep * 2).min(max_idle_sleep);
                    }
                }
            }
        }
    }
}

/// Visibility delay before the next attempt, doubled per failed read.
fn retry_delay_secs(read_ct: ReadCt) -> i32 {
    let doublings = (read_ct.0 - 1).clamp(0, 30) as u32;
    RETRY_BASE_DELAY_SECS
        .saturating_mul(1 << doublings)
        .min(MAX_RETRY_DELAY_SECS)
}

/// A batch-level error would otherwise hold every message of the batch hostage to one
/// poison message: retry them one by one so each outcome is attributed to its message.
async fn handle_isolating_failures(
    queue: PgmqQueue,
    handler: &dyn PgmqHandler,
    msgs: &[PgmqMessage],
) -> HandleResult {
    let err = match handler.handle(msgs).await {
        Ok(result) => return result,
        Err(err) => err,
    };

    if let [msg] = msgs {
        return HandleResult {
            succeeded: vec![],
            failed: vec![HandleResult::fail(msg.msg_id, &err)],
        };
    }

    log::warn!(
        "[{queue}] Batch of {} failed, retrying messages one by one: {err:?}",
        msgs.len()
    );

    let mut result = HandleResult {
        succeeded: vec![],
        failed: vec![],
    };
    for msg in msgs {
        match handler.handle(std::slice::from_ref(msg)).await {
            Ok(single) => {
                result.succeeded.extend(single.succeeded);
                result.failed.extend(single.failed);
            }
            Err(err) => result.failed.push(HandleResult::fail(msg.msg_id, &err)),
        }
    }
    result
}

pub async fn run_once(
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

    // Exhausted messages still go through the handler so the final attempt's error is captured.
    let handle_result = handle_isolating_failures(queue, handler.as_ref(), &messages).await;

    let succeeded_ids: HashSet<i64> = handle_result.succeeded.iter().map(|id| id.0).collect();
    let failed_errors: HashMap<i64, &str> = handle_result
        .failed
        .iter()
        .map(|(id, err)| (id.0, err.as_str()))
        .collect();

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

    if !handle_result.succeeded.is_empty() {
        let to_ack = handle_result.succeeded.clone();
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

    let (to_dead_letter, to_retry): (Vec<&PgmqMessage>, Vec<&PgmqMessage>) = messages
        .iter()
        .filter(|m| !succeeded_ids.contains(&m.msg_id.0))
        .partition(|m| m.read_ct > max_read_count);

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
                    headers: headers_json,
                    read_ct: msg.read_ct.0,
                    enqueued_at: msg.enqueued_at,
                    last_error: Some(last_error),
                }
            })
            .collect();

        let dlq_ids: Vec<MessageId> = to_dead_letter.iter().map(|m| m.msg_id).collect();
        let count = dlq_ids.len();

        store
            .dead_letter_messages(queue, dlq_entries)
            .await
            .change_context(PgmqError::DeadLetterMessages)?;

        log::warn!("[{queue}] Dead-lettered {count} message(s): {dlq_ids:?}");
        MESSAGES_DEAD_LETTERED.add(count as u64, &[KeyValue::new("queue", queue.as_str())]);
    }

    let backoffs: Vec<(MessageId, i32)> = to_retry
        .iter()
        .map(|m| (m.msg_id, retry_delay_secs(m.read_ct)))
        .collect();

    if !backoffs.is_empty() {
        store
            .pgmq_set_vt(queue, backoffs)
            .await
            .change_context(PgmqError::SetVisibilityTimeout)?;
    }

    Ok(MessageReadQty(read_len as i16))
}

pub struct Noop;

#[async_trait::async_trait]
impl PgmqHandler for Noop {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        Ok(HandleResult::from_succeeded(
            msgs.iter().map(|x| x.msg_id).collect_vec(),
        ))
    }
}

pub fn processor_name(prefix: &str) -> String {
    format!("{}-{}", prefix, rand::rng().random::<u16>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::pgmq::Message;

    fn msg(id: i64, poison: bool) -> PgmqMessage {
        PgmqMessage {
            msg_id: MessageId(id),
            message: Some(Message(serde_json::json!({ "poison": poison }))),
            headers: None,
            read_ct: ReadCt(1),
            enqueued_at: chrono::Utc::now().naive_utc(),
        }
    }

    /// Fails the whole batch as soon as it contains a poison message, like a `?` on decode.
    struct BatchFailingHandler;

    #[async_trait::async_trait]
    impl PgmqHandler for BatchFailingHandler {
        async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
            if msgs
                .iter()
                .any(|m| m.message.as_ref().unwrap().0["poison"] == true)
            {
                return Err(error_stack::Report::new(PgmqError::HandleMessages));
            }
            Ok(HandleResult::from_succeeded(
                msgs.iter().map(|m| m.msg_id).collect(),
            ))
        }
    }

    #[tokio::test]
    async fn poison_message_does_not_hold_the_batch_hostage() {
        let msgs = [msg(1, false), msg(2, true), msg(3, false)];

        let result =
            handle_isolating_failures(PgmqQueue::WebhookOut, &BatchFailingHandler, &msgs).await;

        assert_eq!(result.succeeded, vec![MessageId(1), MessageId(3)]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].0, MessageId(2));
    }

    #[tokio::test]
    async fn single_message_batch_error_is_reported_as_failure() {
        let result =
            handle_isolating_failures(PgmqQueue::WebhookOut, &BatchFailingHandler, &[msg(7, true)])
                .await;

        assert!(result.succeeded.is_empty());
        assert_eq!(result.failed[0].0, MessageId(7));
    }

    #[test]
    fn retry_delay_doubles_per_read_and_is_capped() {
        assert_eq!(retry_delay_secs(ReadCt(1)), 15);
        assert_eq!(retry_delay_secs(ReadCt(2)), 30);
        assert_eq!(retry_delay_secs(ReadCt(5)), 240);
        assert_eq!(retry_delay_secs(ReadCt(12)), MAX_RETRY_DELAY_SECS);
        assert_eq!(retry_delay_secs(ReadCt(i32::MAX)), MAX_RETRY_DELAY_SECS);
    }
}
