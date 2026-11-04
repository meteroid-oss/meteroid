use crate::config::{ClickhouseConfig, ConsumerConfig, KafkaConfig};
use crate::ingest::domain::{RawEvent, RawEventRow};
use clickhouse::sql::Identifier;
use clickhouse::{Client, Row};
use kafka::consumer::create_kafka_consumer_with_context;
use rdkafka::consumer::{BaseConsumer, CommitMode, Consumer, ConsumerContext, Rebalance};
use rdkafka::message::Message;
use rdkafka::{ClientContext, Offset, TopicPartitionList};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::{self, Instant};
use uuid::Uuid;

const CONSUMER_GROUP_ID: &str = "clickhouse-events-raw";
const BATCH_MAX_ROWS: usize = 2000;
const BATCH_PERIOD: Duration = Duration::from_millis(500);
const RESTART_DELAY: Duration = Duration::from_secs(5);
/// Keeps each id lookup well under ClickHouse's 256 KiB `max_query_size`.
const LOOKUP_QUERY_BYTES: usize = 64 * 1024;
/// Client-side bound on one lookup; clickhouse-rs sets no request timeout.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
const INSERT_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);

pub async fn run(
    kafka_config: &KafkaConfig,
    clickhouse_config: &ClickhouseConfig,
    consumer_config: &ConsumerConfig,
) {
    loop {
        if let Err(e) = run_inner(kafka_config, clickhouse_config, consumer_config).await {
            log::error!(
                "Kafka consumer error, restarting in {}s: {e:?}",
                RESTART_DELAY.as_secs()
            );
            time::sleep(RESTART_DELAY).await;
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ConsumerError {
    #[error("ClickHouse error: {0}")]
    Clickhouse(#[from] clickhouse::error::Error),
    #[error("Kafka error: {0}")]
    Kafka(#[from] rdkafka::error::KafkaError),
    #[error("ClickHouse request timed out after {0:?}")]
    Timeout(Duration),
    #[error("{0}")]
    Config(String),
}

/// Kafka evicts a member that does not poll for `max.poll.interval.ms` (librdkafka default).
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(300);

fn check_timing(config: &ConsumerConfig) -> Result<(), ConsumerError> {
    let attempt = Duration::from_millis(config.insert_quorum_timeout_ms) * 2;
    let budget = Duration::from_millis(config.insert_retry_budget_ms);
    if attempt.is_zero() || budget.max(attempt) + LOOKUP_TIMEOUT >= MAX_POLL_INTERVAL {
        return Err(ConsumerError::Config(format!(
            "insert quorum timeout ({attempt:?} per attempt) and retry budget ({budget:?}) must be \
             non-zero and fit, with the lookup, under {MAX_POLL_INTERVAL:?}"
        )));
    }
    Ok(())
}

/// Raised on every rebalance, so the loop drops its batch. Under eager assignment (librdkafka's
/// default) every partition restarts from its committed offset, so the dropped events come back to
/// whichever consumer now owns them.
struct RebalanceFlag(Arc<AtomicBool>);

impl ClientContext for RebalanceFlag {}

impl ConsumerContext for RebalanceFlag {
    fn post_rebalance(&self, _: &BaseConsumer<Self>, rebalance: &Rebalance<'_>) {
        if let Rebalance::Assign(_) | Rebalance::Revoke(_) = rebalance {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Debug, Deserialize, Row)]
struct SeenId {
    #[serde(with = "clickhouse::serde::uuid")]
    tenant_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    customer_id: Uuid,
    id: String,
}

type EventKey = (Uuid, Uuid, String);
type LookupKey = (String, String, String);
type Offsets = HashMap<(String, i32), Offset>;

#[derive(Default)]
struct Batch {
    events: Vec<RawEvent>,
    offsets: Offsets,
}

impl Batch {
    fn discard(&mut self) {
        if !self.offsets.is_empty() {
            log::info!(
                "Rebalanced, dropping {} unflushed events to re-read them from the committed offsets",
                self.events.len()
            );
        }
        *self = Batch::default();
    }
}

fn key(event: &RawEvent) -> EventKey {
    (*event.tenant_id, *event.customer_id, event.id.clone())
}

async fn run_inner(
    kafka_config: &KafkaConfig,
    clickhouse_config: &ClickhouseConfig,
    consumer_config: &ConsumerConfig,
) -> Result<(), ConsumerError> {
    check_timing(consumer_config)?;
    let rebalanced = Arc::new(AtomicBool::new(false));
    let consumer = create_kafka_consumer_with_context(
        &kafka_config.kafka_connection,
        &[&kafka_config.kafka_raw_topic],
        CONSUMER_GROUP_ID,
        RebalanceFlag(rebalanced.clone()),
    );

    let client = Client::default()
        .with_url(&clickhouse_config.http_address)
        .with_user(&clickhouse_config.username)
        .with_password(&clickhouse_config.password)
        .with_database(&clickhouse_config.database);
    let quorum = insert_quorum(&client, consumer_config).await?;
    log::info!("Consumer inserts with insert_quorum={quorum}");
    let client = client.with_setting("insert_quorum", quorum).with_setting(
        "insert_quorum_timeout",
        consumer_config.insert_quorum_timeout_ms.to_string(),
    );

    let cluster = &clickhouse_config.cluster_name;
    let mut batch = Batch::default();
    let commit = |offsets: Offsets| -> Result<(), ConsumerError> {
        consumer.commit(
            &TopicPartitionList::from_topic_map(&offsets)?,
            CommitMode::Async,
        )?;
        Ok(())
    };

    let mut interval = time::interval(BATCH_PERIOD);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg_result = consumer.recv() => {
                let msg = msg_result?;
                let position = (msg.topic().to_string(), msg.partition());
                let next_offset = Offset::Offset(msg.offset() + 1);
                let event = msg.payload().and_then(|payload| {
                    serde_json::from_slice::<RawEvent>(payload)
                        .inspect_err(|e| log::warn!("Failed to deserialize event at partition={} and offset={}, skipping: {e:?}", msg.partition(), msg.offset()))
                        .ok()
                });
                drop(msg);

                if rebalanced.swap(false, Ordering::SeqCst) {
                    batch.discard();
                }

                batch.events.extend(event);
                batch.offsets.insert(position, next_offset);

                if batch.events.len() >= BATCH_MAX_ROWS {
                    flush(&client, cluster, consumer_config, &mut batch, commit).await?;
                }
            }

            _ = interval.tick() => {
                if rebalanced.swap(false, Ordering::SeqCst) {
                    batch.discard();
                } else {
                    flush(&client, cluster, consumer_config, &mut batch, commit).await?;
                }
            }
        }
    }
}

/// ClickHouse's own `auto` waits for a quorum that never completes on a single replica (25.6, 25.8),
/// so `auto` is resolved here, at each consumer start, to a majority of the raw table's replicas.
/// A replica without its Keeper session reports 0 replicas: refuse to start rather than use 1.
async fn insert_quorum(client: &Client, config: &ConsumerConfig) -> Result<String, ConsumerError> {
    if config.insert_quorum != "auto" {
        return Ok(config.insert_quorum.clone());
    }
    let (total, active, expired) = client
        .query("SELECT toUInt32(total_replicas), toUInt32(active_replicas), toUInt8(is_session_expired) FROM system.replicas WHERE database = currentDatabase() AND table = ?")
        .bind(&config.raw_events_local_table)
        .fetch_one::<(u32, u32, u8)>()
        .await?;
    log::info!(
        "{} replicas: {total} total, {active} active",
        config.raw_events_local_table
    );
    if total == 0 || expired != 0 {
        return Err(ConsumerError::Config(
            "replica has no Keeper session, cannot resolve the insert quorum".to_string(),
        ));
    }
    Ok(quorum_for(total).to_string())
}

fn quorum_for(replicas: u32) -> u32 {
    replicas / 2 + 1
}

/// Drops already-seen events, inserts the rest in one insert, then commits the offsets.
async fn flush(
    client: &Client,
    cluster: &str,
    config: &ConsumerConfig,
    batch: &mut Batch,
    commit: impl FnOnce(Offsets) -> Result<(), ConsumerError>,
) -> Result<(), ConsumerError> {
    if batch.offsets.is_empty() {
        return Ok(());
    }

    let Batch { events, offsets } = std::mem::take(batch);
    let received = events.len();
    let events = dedup_in_batch(events);
    let seen = lookup_seen(client, cluster, &config.event_ids_local_table, &events).await?;
    let rows: Vec<RawEventRow> = drop_seen(events, &seen)
        .into_iter()
        .map(RawEventRow::from)
        .collect();

    if !rows.is_empty() {
        insert_with_retries(client, config, &rows).await?;
    }

    commit(offsets)?;

    log::info!(
        "Flushed {} raw events to ClickHouse, dropped {} duplicates",
        rows.len(),
        received - rows.len()
    );
    Ok(())
}

/// Retries the byte-identical insert: ClickHouse drops a block it already stored and still feeds
/// the view, so an insert of unknown status (quorum timeout) gets its ids instead of being stored
/// twice by a rebuilt batch. The budget stays under Kafka's 300 s `max.poll.interval.ms`.
async fn insert_with_retries(
    client: &Client,
    config: &ConsumerConfig,
    rows: &[RawEventRow],
) -> Result<(), ConsumerError> {
    let attempt_timeout = Duration::from_millis(config.insert_quorum_timeout_ms) * 2;
    let deadline = Instant::now() + Duration::from_millis(config.insert_retry_budget_ms);
    let mut delay = Duration::from_secs(1);
    loop {
        let result = time::timeout(attempt_timeout, insert_rows(client, config, rows))
            .await
            .unwrap_or(Err(ConsumerError::Timeout(attempt_timeout)));
        match result {
            Err(e) if Instant::now() + delay + attempt_timeout < deadline => {
                log::warn!("Insert failed, retrying the same batch in {delay:?}: {e:?}");
                time::sleep(delay).await;
                delay = (delay * 2).min(INSERT_RETRY_MAX_DELAY);
            }
            result => return result,
        }
    }
}

async fn insert_rows(
    client: &Client,
    config: &ConsumerConfig,
    rows: &[RawEventRow],
) -> Result<(), ConsumerError> {
    let mut insert = client
        .insert::<RawEventRow>(&config.raw_events_local_table)
        .await?;
    for row in rows {
        insert.write(row).await?;
    }
    insert.end().await?;
    Ok(())
}

/// Keeps the first copy of each key; keep-last events are never dropped.
fn dedup_in_batch(events: Vec<RawEvent>) -> Vec<RawEvent> {
    let mut keys = HashSet::new();
    events
        .into_iter()
        .filter(|event| event.keep_last || keys.insert(key(event)))
        .collect()
}

fn drop_seen(events: Vec<RawEvent>, seen: &HashSet<EventKey>) -> Vec<RawEvent> {
    events
        .into_iter()
        .filter(|event| event.keep_last || !seen.contains(&key(event)))
        .collect()
}

async fn lookup_seen(
    client: &Client,
    cluster: &str,
    ids_table: &str,
    events: &[RawEvent],
) -> Result<HashSet<EventKey>, ConsumerError> {
    let keys = events
        .iter()
        .filter(|event| !event.keep_last)
        .map(|event| {
            let (tenant_id, customer_id, id) = key(event);
            (tenant_id.to_string(), customer_id.to_string(), id)
        })
        .collect();

    // Concurrent, so a slow replica costs one handshake per batch, not one per chunk.
    let lookups = chunk_keys(keys, LOOKUP_QUERY_BYTES).into_iter().map(|chunk| async move {
        // Quorum inserts put every acknowledged id on a majority of replicas, so the reachable
        // replicas still see it with one down. Redundant on 2 replicas, but 3 is then config-only.
        let query = client
            .query("SELECT ?fields FROM clusterAllReplicas(?, currentDatabase(), ?) WHERE (tenant_id, customer_id, id) IN ?")
            .bind(cluster)
            .bind(Identifier(ids_table))
            .bind(chunk)
            .with_setting("skip_unavailable_shards", "1")
            .with_setting("connect_timeout_with_failover_ms", "500")
            .with_setting("connections_with_failover_max_tries", "1")
            .with_setting("handshake_timeout_ms", "2000")
            .with_setting("max_execution_time", "5")
            // The raised limit only matters for a single key longer than a chunk.
            .with_setting("max_query_size", (4 * 1024 * 1024).to_string())
            .fetch_all::<SeenId>();
        time::timeout(LOOKUP_TIMEOUT, query)
            .await
            .map_err(|_| ConsumerError::Timeout(LOOKUP_TIMEOUT))?
            .map_err(ConsumerError::from)
    });

    Ok(futures::future::try_join_all(lookups)
        .await?
        .into_iter()
        .flatten()
        .map(|hit| (hit.tenant_id, hit.customer_id, hit.id))
        .collect())
}

/// Splits keys so each chunk's SQL literal stays under `max_bytes`, counting the id's escaping at
/// worst (UUIDs need none).
fn chunk_keys(keys: Vec<LookupKey>, max_bytes: usize) -> Vec<Vec<LookupKey>> {
    let mut chunks: Vec<Vec<LookupKey>> = Vec::new();
    let mut size = 0;
    for key in keys {
        let key_size = key.0.len() + key.1.len() + 2 * key.2.len() + 11;
        if chunks.is_empty() || (size > 0 && size + key_size > max_bytes) {
            chunks.push(Vec::new());
            size = 0;
        }
        size += key_size;
        chunks.last_mut().expect("pushed above").push(key);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::{CustomerId, TenantId};

    fn event(customer: u128, id: &str, value: &str, keep_last: bool) -> RawEvent {
        let at = chrono::Utc::now().naive_utc();
        RawEvent {
            id: id.to_string(),
            code: "calls".to_string(),
            customer_id: CustomerId::from(Uuid::from_u128(customer)),
            tenant_id: TenantId::from(Uuid::from_u128(100)),
            timestamp: at,
            ingested_at: at,
            properties: HashMap::from([("value".to_string(), value.to_string())]),
            keep_last,
        }
    }

    fn values(events: &[RawEvent]) -> Vec<&str> {
        events
            .iter()
            .map(|e| e.properties["value"].as_str())
            .collect()
    }

    #[test]
    fn in_batch_dedup_keeps_the_first_copy_per_customer_and_id() {
        let batch = vec![
            event(1, "a", "first", false),
            event(1, "a", "second", false),
            event(2, "a", "other customer", false),
            event(1, "b", "other id", false),
        ];

        assert_eq!(
            values(&dedup_in_batch(batch)),
            ["first", "other customer", "other id"]
        );
    }

    #[test]
    fn keep_last_events_bypass_both_drops() {
        let batch = vec![event(1, "a", "first", true), event(1, "a", "second", true)];
        let batch = dedup_in_batch(batch);
        let seen = HashSet::from([key(&batch[0])]);

        assert_eq!(values(&drop_seen(batch, &seen)), ["first", "second"]);
    }

    #[test]
    fn seen_ids_are_dropped() {
        let batch = vec![event(1, "a", "seen", false), event(1, "b", "new", false)];
        let seen = HashSet::from([key(&batch[0])]);

        assert_eq!(values(&drop_seen(batch, &seen)), ["new"]);
    }

    #[test]
    fn lookup_chunks_stay_under_the_budget() {
        let uuid = Uuid::from_u128(1).to_string();
        let keys: Vec<_> = (0..2000)
            .map(|i| {
                (
                    uuid.clone(),
                    uuid.clone(),
                    format!("{i:04}{}", "€".repeat(251)),
                )
            })
            .collect();

        let chunks = chunk_keys(keys.clone(), LOOKUP_QUERY_BYTES);

        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), keys);
        for chunk in &chunks {
            let literal: usize = chunk
                .iter()
                .map(|(t, c, id)| t.len() + c.len() + 2 * id.len() + 11)
                .sum();
            assert!(literal <= LOOKUP_QUERY_BYTES, "{literal}");
        }
        let oversized = vec![(uuid.clone(), uuid, "x".repeat(LOOKUP_QUERY_BYTES))];
        assert_eq!(
            chunk_keys(oversized.clone(), LOOKUP_QUERY_BYTES),
            [oversized]
        );
    }

    fn unreachable_clickhouse() -> (Client, ConsumerConfig) {
        let config = ConsumerConfig {
            insert_quorum: "auto".to_string(),
            insert_quorum_timeout_ms: 30_000,
            insert_retry_budget_ms: 0,
            raw_events_local_table: "raw_events_local_v2".to_string(),
            event_ids_local_table: "event_ids_local".to_string(),
        };
        (Client::default().with_url("http://127.0.0.1:1"), config)
    }

    fn pending(events: Vec<RawEvent>) -> Batch {
        Batch {
            events,
            offsets: HashMap::from([(("raw".to_string(), 0), Offset::Offset(2))]),
        }
    }

    #[tokio::test]
    async fn a_failed_lookup_or_insert_commits_nothing() {
        let (client, config) = unreachable_clickhouse();
        let looked_up = pending(vec![event(1, "a", "v", false)]);
        let inserted_only = pending(vec![event(1, "a", "v", true)]);

        for mut batch in [looked_up, inserted_only] {
            let mut committed = None;
            let result = flush(&client, "meteroid", &config, &mut batch, |offsets| {
                committed = Some(offsets);
                Ok(())
            })
            .await;

            assert!(result.is_err());
            assert!(committed.is_none());
        }
    }

    #[test]
    fn quorum_is_a_majority() {
        assert_eq!([1, 2, 3].map(quorum_for), [1, 2, 2]);
    }

    #[test]
    fn timing_must_fit_under_the_poll_interval() {
        let (_, mut config) = unreachable_clickhouse();
        config.insert_retry_budget_ms = 240_000;
        assert!(check_timing(&config).is_ok());
        config.insert_quorum_timeout_ms = 0;
        assert!(check_timing(&config).is_err());
        config.insert_quorum_timeout_ms = 150_000;
        assert!(check_timing(&config).is_err());
    }

    #[tokio::test]
    async fn a_failed_insert_is_retried_within_the_budget() {
        let (client, mut config) = unreachable_clickhouse();
        config.insert_quorum_timeout_ms = 100;
        config.insert_retry_budget_ms = 1_500;
        let rows = [RawEventRow::from(event(1, "a", "v", true))];

        let started = Instant::now();
        assert!(insert_with_retries(&client, &config, &rows).await.is_err());
        assert!(started.elapsed() >= Duration::from_secs(1));
    }

    #[test]
    fn a_rebalance_drops_the_batch_and_its_offsets() {
        let mut batch = pending(vec![event(1, "a", "v", false)]);

        batch.discard();

        assert!(batch.events.is_empty());
        assert!(batch.offsets.is_empty());
    }

    #[test]
    fn keep_last_flag_defaults_to_false_on_the_wire() {
        let mut json = serde_json::to_value(event(1, "a", "v", true)).unwrap();
        json.as_object_mut().unwrap().remove("keep_last");

        assert!(!serde_json::from_value::<RawEvent>(json).unwrap().keep_last);
    }
}
