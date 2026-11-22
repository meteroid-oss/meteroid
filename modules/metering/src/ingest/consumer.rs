use crate::config::{ClickhouseConfig, ConsumerConfig, KafkaConfig};
use crate::connectors::clickhouse::event_rows::{self, LookupKey, StoredRow};
use crate::ingest::domain::{EventKind, RawEvent, RawEventRow};
use chrono::{TimeZone, Utc};
use clickhouse::sql::Identifier;
use clickhouse::{Client, Row};
use kafka::consumer::create_kafka_consumer_with_context;
use rdkafka::consumer::{BaseConsumer, CommitMode, Consumer, ConsumerContext, Rebalance};
use rdkafka::message::Message;
use rdkafka::{ClientContext, Offset, TopicPartitionList};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
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
    #[error("Insert retry budget exhausted")]
    Budget,
    #[error("{0}")]
    Config(String),
}

/// Kafka evicts a member that does not poll for `max.poll.interval.ms` (librdkafka default).
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(300);

fn check_timing(config: &ConsumerConfig) -> Result<(), ConsumerError> {
    let attempt = Duration::from_millis(config.insert_quorum_timeout_ms) * 2;
    let budget = Duration::from_millis(config.insert_retry_budget_ms);
    if attempt.is_zero() || budget.max(attempt) + 2 * LOOKUP_TIMEOUT >= MAX_POLL_INTERVAL {
        return Err(ConsumerError::Config(format!(
            "insert quorum timeout ({attempt:?} per attempt) and retry budget ({budget:?}) must be \
             non-zero and fit, with the id and change lookups, under {MAX_POLL_INTERVAL:?}"
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
type Offsets = HashMap<(String, i32), Offset>;

#[derive(Default)]
struct Batch {
    events: Vec<RawEvent>,
    /// Amends and voids, in Kafka order; they apply once the batch's events are stored.
    changes: Vec<RawEvent>,
    offsets: Offsets,
}

impl Batch {
    fn discard(&mut self) {
        if !self.offsets.is_empty() {
            log::info!(
                "Rebalanced, dropping {} unflushed events and {} changes to re-read them from the \
                 committed offsets",
                self.events.len(),
                self.changes.len()
            );
        }
        *self = Batch::default();
    }

    /// True when the batch is full and must flush now.
    fn push(&mut self, event: Option<RawEvent>, position: (String, i32), offset: Offset) -> bool {
        match event {
            Some(change) if change.kind != EventKind::Event => self.changes.push(change),
            event => self.events.extend(event),
        }
        self.offsets.insert(position, offset);
        self.events.len() + self.changes.len() >= BATCH_MAX_ROWS
    }
}

fn key(event: &RawEvent) -> EventKey {
    (*event.tenant_id, *event.customer_id, event.id.clone())
}

fn lookup_key(event: &RawEvent) -> LookupKey {
    let (tenant_id, customer_id, id) = key(event);
    (tenant_id.to_string(), customer_id.to_string(), id)
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
    let cluster = &clickhouse_config.cluster_name;
    event_rows::check_access(&client, cluster, &consumer_config.tables)
        .await
        .map_err(|e| {
            ConsumerError::Config(format!(
                "the consumer user needs REMOTE and SELECT grants (ADR-0012 rollout): {e}"
            ))
        })?;
    let quorum = insert_quorum(&client, consumer_config).await?;
    log::info!("Consumer inserts with insert_quorum={quorum}");
    let client = client.with_setting("insert_quorum", quorum).with_setting(
        "insert_quorum_timeout",
        consumer_config.insert_quorum_timeout_ms.to_string(),
    );

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

                if batch.push(event, position, next_offset) {
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
        .bind(&config.tables.raw_events_local_table)
        .fetch_one::<(u32, u32, u8)>()
        .await?;
    log::info!(
        "{} replicas: {total} total, {active} active",
        config.tables.raw_events_local_table
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

/// Drops already-seen events, inserts the rest in one insert, applies the batch's changes, then
/// commits the offsets.
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

    let Batch {
        events,
        changes,
        offsets,
    } = std::mem::take(batch);
    let mut budget = InsertBudget::new(config);
    let received = events.len();
    let events = dedup_in_batch(events);
    let seen = lookup_seen(
        client,
        cluster,
        &config.tables.event_ids_local_table,
        &events,
    )
    .await?;
    let rows: Vec<RawEventRow> = drop_seen(events, &seen)
        .into_iter()
        .map(RawEventRow::from)
        .collect();

    if !rows.is_empty() {
        insert_with_retries(client, config, &rows, &mut budget).await?;
    }
    if !changes.is_empty() {
        apply_changes(client, cluster, config, &changes, &mut budget).await?;
    }

    commit(offsets)?;

    log::info!(
        "Flushed {} raw events and {} changes to ClickHouse, dropped {} duplicates",
        rows.len(),
        changes.len(),
        received - rows.len()
    );
    Ok(())
}

/// One flush's inserts share the retry budget; an insert after the first starts only if its attempt
/// fits, so a flush stays under Kafka's 300 s `max.poll.interval.ms`.
struct InsertBudget {
    deadline: Instant,
    attempt: Duration,
    first: bool,
}

impl InsertBudget {
    fn new(config: &ConsumerConfig) -> Self {
        InsertBudget {
            deadline: Instant::now() + Duration::from_millis(config.insert_retry_budget_ms),
            attempt: Duration::from_millis(config.insert_quorum_timeout_ms) * 2,
            first: true,
        }
    }

    fn allows(&mut self, delay: Duration) -> bool {
        std::mem::take(&mut self.first) || Instant::now() + delay + self.attempt < self.deadline
    }
}

/// Retries the same rows, which serialize to the same block (sorted properties): ClickHouse drops a
/// block it already stored and still feeds the view, so an insert of unknown status (quorum
/// timeout) gets its ids instead of being stored twice.
async fn insert_with_retries(
    client: &Client,
    config: &ConsumerConfig,
    rows: &[RawEventRow],
    budget: &mut InsertBudget,
) -> Result<(), ConsumerError> {
    if !budget.allows(Duration::ZERO) {
        return Err(ConsumerError::Budget);
    }
    let mut delay = Duration::from_secs(1);
    loop {
        let result = time::timeout(budget.attempt, insert_rows(client, config, rows))
            .await
            .unwrap_or(Err(ConsumerError::Timeout(budget.attempt)));
        match result {
            Err(e) if budget.allows(delay) => {
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
        .insert::<RawEventRow>(&config.tables.raw_events_local_table)
        .await?;
    for row in rows {
        insert.write(row).await?;
    }
    insert.end().await?;
    Ok(())
}

/// Writes what the amends and voids still miss (ADR-0012 §4), after one read of their keys' copies.
async fn apply_changes(
    client: &Client,
    cluster: &str,
    config: &ConsumerConfig,
    changes: &[RawEvent],
    budget: &mut InsertBudget,
) -> Result<(), ConsumerError> {
    let keys: HashSet<LookupKey> = changes.iter().map(lookup_key).collect();
    let lookups = chunk_keys(keys.into_iter().collect(), LOOKUP_QUERY_BYTES)
        .into_iter()
        .map(|chunk| async move {
            bounded(event_rows::lookup(client, cluster, &config.tables, &chunk)).await
        });
    let mut stored: HashMap<EventKey, Vec<StoredRow>> = HashMap::new();
    for row in futures::future::try_join_all(lookups)
        .await?
        .into_iter()
        .flatten()
    {
        let key = (row.tenant_id, row.customer_id, row.id.clone());
        stored.entry(key).or_default().push(row);
    }
    for insert in plan_changes(changes, stored).inserts() {
        insert_with_retries(client, config, &insert, budget).await?;
    }
    Ok(())
}

/// Applies each change to its key's copies in Kafka order, the next change seeing what the
/// previous one wrote.
fn plan_changes(changes: &[RawEvent], mut stored: HashMap<EventKey, Vec<StoredRow>>) -> ChangeRows {
    let mut plan = ChangeRows::default();
    for change in changes {
        let copies = stored.entry(key(change)).or_default();
        match change_rows(change, copies) {
            Ok(rows) => {
                log::info!(
                    "Applying {:?} of event {}: {} new, {} tombstones",
                    change.kind,
                    change.id,
                    rows.new_rows.len(),
                    rows.tombstones.len()
                );
                for tombstone in &rows.tombstones {
                    copies
                        .iter_mut()
                        .filter(|copy| copy.ingested_at == tombstone.ingested_at)
                        .for_each(|copy| copy.voided = 1);
                }
                copies.extend(rows.new_rows.iter().map(stored_row));
                plan.new_rows.extend(rows.new_rows);
                plan.tombstones.extend(rows.tombstones);
            }
            Err(reason) => log::warn!(
                "Dropped {:?} of event {} (tenant {}, customer {}): {reason}",
                change.kind,
                change.id,
                change.tenant_id,
                change.customer_id
            ),
        }
    }
    plan
}

fn stored_row(row: &RawEventRow) -> StoredRow {
    StoredRow {
        tenant_id: row.tenant_id,
        customer_id: row.customer_id,
        id: row.id.clone(),
        code: row.code.clone(),
        timestamp: row.timestamp,
        properties: row.properties.clone(),
        ingested_at: row.ingested_at,
        voided: 0,
    }
}

#[derive(Debug, Default, PartialEq)]
struct ChangeRows {
    new_rows: Vec<RawEventRow>,
    tombstones: Vec<RawEventRow>,
}

impl ChangeRows {
    /// One insert, unless the rows fall in several months (several parts): the new rows are then
    /// stored first, so no replay finds an old row voided without its successor.
    fn inserts(self) -> Vec<Vec<RawEventRow>> {
        let month = |row: &RawEventRow| row.timestamp.format("%Y%m").to_string();
        let months: HashSet<String> = self
            .new_rows
            .iter()
            .chain(&self.tombstones)
            .map(month)
            .collect();
        if months.len() > 1 && !self.new_rows.is_empty() && !self.tombstones.is_empty() {
            return vec![self.new_rows, self.tombstones];
        }
        let rows: Vec<RawEventRow> = self.new_rows.into_iter().chain(self.tombstones).collect();
        if rows.is_empty() { vec![] } else { vec![rows] }
    }
}

/// A void tombstones every live copy; an amend every live copy but its own (stamped with its
/// `ingested_at`) and adds its own if missing. Under first-wins a copy stored after the stamp can
/// only be a later change's new row, so the amend is then superseded, as when its own copy is voided.
fn change_rows(change: &RawEvent, stored: &[StoredRow]) -> Result<ChangeRows, &'static str> {
    let amend = change.kind == EventKind::Amend;
    let stamp = Utc.from_utc_datetime(&change.ingested_at);
    let own = |row: &&StoredRow| amend && row.ingested_at == stamp;
    let later = |row: &StoredRow| row.ingested_at > stamp || (own(&row) && row.voided != 0);
    if amend && stored.iter().any(later) {
        return Err("superseded by a later change");
    }
    let live: Vec<&StoredRow> = stored.iter().filter(|row| row.voided == 0).collect();
    let first = live.first().ok_or("no live copy")?;
    let row = |code: &str, timestamp, ingested_at, properties: &BTreeMap<_, _>, is_tombstone| {
        RawEventRow {
            id: change.id.clone(),
            code: code.to_string(),
            customer_id: *change.customer_id,
            tenant_id: *change.tenant_id,
            timestamp,
            ingested_at,
            properties: properties.clone(),
            is_tombstone,
        }
    };
    let new_row = (amend && !live.iter().any(own)).then(|| {
        let timestamp = Utc.from_utc_datetime(&change.timestamp);
        let properties = change.properties.clone().into_iter().collect();
        row(&first.code, timestamp, stamp, &properties, 0)
    });
    Ok(ChangeRows {
        new_rows: new_row.into_iter().collect(),
        tombstones: live
            .iter()
            .filter(|stored| !own(stored))
            .map(|r| row(&r.code, r.timestamp, r.ingested_at, &r.properties, 1))
            .collect(),
    })
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
        .map(lookup_key)
        .collect();

    // Concurrent, so a slow replica costs one handshake per batch, not one per chunk.
    let lookups = chunk_keys(keys, LOOKUP_QUERY_BYTES).into_iter().map(|chunk| async move {
        // Quorum inserts put every acknowledged id on a majority of replicas, so the reachable
        // replicas still see it with one down. Redundant on 2 replicas, but 3 is then config-only.
        let query = client
            .query("SELECT ?fields FROM clusterAllReplicas(?, currentDatabase(), ?) WHERE (tenant_id, customer_id, id) IN ?")
            .bind(cluster)
            .bind(Identifier(ids_table))
            .bind(chunk);
        bounded(event_rows::across_replicas(query).fetch_all::<SeenId>()).await
    });

    Ok(futures::future::try_join_all(lookups)
        .await?
        .into_iter()
        .flatten()
        .map(|hit| (hit.tenant_id, hit.customer_id, hit.id))
        .collect())
}

async fn bounded<T>(
    lookup: impl Future<Output = Result<T, clickhouse::error::Error>>,
) -> Result<T, ConsumerError> {
    time::timeout(LOOKUP_TIMEOUT, lookup)
        .await
        .map_err(|_| ConsumerError::Timeout(LOOKUP_TIMEOUT))?
        .map_err(ConsumerError::from)
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
            kind: EventKind::Event,
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
            tables: crate::config::EventTablesConfig {
                raw_events_local_table: "raw_events_local_v2".to_string(),
                event_ids_local_table: "event_ids_local".to_string(),
            },
        };
        (Client::default().with_url("http://127.0.0.1:1"), config)
    }

    fn pending(events: Vec<RawEvent>) -> Batch {
        Batch {
            events,
            changes: vec![],
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
        let mut budget = InsertBudget::new(&config);
        assert!(
            insert_with_retries(&client, &config, &rows, &mut budget)
                .await
                .is_err()
        );
        assert!(started.elapsed() >= Duration::from_secs(1));
    }

    #[test]
    fn a_rebalance_drops_the_batch_and_its_offsets() {
        let mut batch = pending(vec![event(1, "a", "v", false)]);

        batch.discard();

        assert!(batch.events.is_empty());
        assert!(batch.offsets.is_empty());
    }

    #[tokio::test]
    async fn an_insert_after_the_first_only_starts_within_the_budget() {
        let (client, mut config) = unreachable_clickhouse();
        config.insert_quorum_timeout_ms = 100;
        let rows = [RawEventRow::from(event(1, "a", "v", true))];
        let mut budget = InsertBudget::new(&config);

        assert!(matches!(
            insert_with_retries(&client, &config, &rows, &mut budget).await,
            Err(ConsumerError::Clickhouse(_))
        ));
        assert!(matches!(
            insert_with_retries(&client, &config, &rows, &mut budget).await,
            Err(ConsumerError::Budget)
        ));
    }

    fn at(day: u32, nanos: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap()
            + chrono::Duration::nanoseconds(nanos.into())
    }

    fn stored(
        value: &str,
        timestamp: chrono::DateTime<Utc>,
        ingested: u32,
        voided: bool,
    ) -> StoredRow {
        StoredRow {
            tenant_id: Uuid::from_u128(100),
            customer_id: Uuid::from_u128(1),
            id: "a".to_string(),
            code: "calls".to_string(),
            timestamp,
            properties: BTreeMap::from([("value".to_string(), value.to_string())]),
            ingested_at: at(1, ingested),
            voided: voided.into(),
        }
    }

    fn change(kind: EventKind, timestamp: chrono::DateTime<Utc>, stamp: u32) -> RawEvent {
        let mut change = event(1, "a", "new", false);
        change.code = String::new();
        change.kind = kind;
        change.timestamp = timestamp.naive_utc();
        change.ingested_at = at(1, stamp).naive_utc();
        change
    }

    fn rows_of(rows: &[RawEventRow]) -> Vec<(&str, u32, u8)> {
        rows.iter()
            .map(|r| {
                let nanos = r.ingested_at.timestamp_subsec_nanos();
                (r.properties["value"].as_str(), nanos, r.is_tombstone)
            })
            .collect()
    }

    #[test]
    fn a_void_tombstones_every_live_copy() {
        let copies = [
            stored("first", at(2, 0), 1, false),
            stored("pre-dedup copy", at(3, 0), 2, false),
            stored("voided", at(2, 0), 3, true),
        ];

        let rows = change_rows(&change(EventKind::Void, at(9, 0), 9), &copies).unwrap();

        assert!(rows.new_rows.is_empty());
        assert_eq!(
            rows_of(&rows.tombstones),
            [("first", 1, 1), ("pre-dedup copy", 2, 1)]
        );
        let tombstone = &rows.tombstones[0];
        let original = RawEventRow::from(event(1, "a", "first", false));
        assert_eq!(
            (
                tombstone.id.as_str(),
                tombstone.code.as_str(),
                tombstone.timestamp
            ),
            (original.id.as_str(), "calls", at(2, 0)),
            "an exact copy of the stored row"
        );
    }

    #[test]
    fn an_amend_tombstones_the_live_copies_and_adds_its_row() {
        let rows = change_rows(
            &change(EventKind::Amend, at(5, 0), 9),
            &[stored("old", at(2, 0), 1, false)],
        )
        .unwrap();

        let new_row = rows.new_rows[0].clone();
        assert_eq!(
            (
                new_row.code.as_str(),
                new_row.timestamp,
                new_row.is_tombstone
            ),
            ("calls", at(5, 0), 0),
            "the stored code, the amend's timestamp"
        );
        assert_eq!(rows_of(&[new_row]), [("new", 9, 0)]);
        assert_eq!(rows_of(&rows.tombstones), [("old", 1, 1)]);
    }

    #[test]
    fn a_replayed_change_writes_only_what_is_missing() {
        let amend = change(EventKind::Amend, at(5, 0), 9);
        let applied = [
            stored("old", at(2, 0), 1, true),
            stored("new", at(5, 0), 9, false),
        ];
        assert_eq!(change_rows(&amend, &applied), Ok(ChangeRows::default()));

        let new_row_only = [
            stored("old", at(2, 0), 1, false),
            stored("new", at(5, 0), 9, false),
        ];
        let repair = change_rows(&amend, &new_row_only).unwrap();
        assert!(repair.new_rows.is_empty());
        assert_eq!(rows_of(&repair.tombstones), [("old", 1, 1)]);

        let void = change(EventKind::Void, at(9, 0), 10);
        let one_voided = [
            stored("a", at(2, 0), 1, true),
            stored("b", at(3, 0), 2, false),
        ];
        assert_eq!(
            rows_of(&change_rows(&void, &one_voided).unwrap().tombstones),
            [("b", 2, 1)]
        );
        assert_eq!(
            change_rows(&void, &[stored("a", at(2, 0), 1, true)]),
            Err("no live copy")
        );
    }

    #[test]
    fn an_amend_replayed_after_a_later_change_is_skipped() {
        let first = change(EventKind::Amend, at(5, 0), 9);
        let after_second = [
            stored("old", at(2, 0), 1, true),
            stored("new", at(5, 0), 9, true),
            stored("newer", at(6, 0), 10, false),
        ];

        assert_eq!(
            change_rows(&first, &after_second),
            Err("superseded by a later change")
        );

        let second_half_applied = [
            stored("old", at(2, 0), 0, true),
            stored("new", at(5, 0), 9, false),
            stored("newer", at(6, 0), 10, false),
        ];
        assert_eq!(
            change_rows(&first, &second_half_applied),
            Err("superseded by a later change"),
            "the later row is a later change's, which must not be tombstoned"
        );
    }

    #[test]
    fn a_change_to_an_unknown_event_is_dropped() {
        for kind in [EventKind::Amend, EventKind::Void] {
            assert_eq!(
                change_rows(&change(kind, at(5, 0), 9), &[]),
                Err("no live copy")
            );
        }
    }

    #[test]
    fn a_new_row_lands_before_tombstones_of_another_month() {
        let same_month = change_rows(
            &change(EventKind::Amend, at(5, 0), 9),
            &[stored("old", at(2, 0), 1, false)],
        )
        .unwrap();
        let inserts = same_month.inserts();
        assert_eq!(inserts.len(), 1);
        assert_eq!(rows_of(&inserts[0]), [("new", 9, 0), ("old", 1, 1)]);

        let september = Utc.with_ymd_and_hms(2026, 9, 30, 0, 0, 0).unwrap();
        let other_month = change_rows(
            &change(EventKind::Amend, at(5, 0), 9),
            &[stored("old", september, 1, false)],
        )
        .unwrap();
        let inserts = other_month.inserts();
        assert_eq!(inserts.len(), 2);
        assert_eq!(rows_of(&inserts[0]), [("new", 9, 0)]);
        assert_eq!(rows_of(&inserts[1]), [("old", 1, 1)]);

        assert!(ChangeRows::default().inserts().is_empty());
    }

    #[test]
    fn an_event_and_its_amend_flush_together_events_first() {
        let mut batch = Batch::default();
        let position = || ("raw".to_string(), 0);

        assert!(!batch.push(
            Some(event(1, "a", "v", false)),
            position(),
            Offset::Offset(1)
        ));
        assert!(!batch.push(
            Some(change(EventKind::Amend, at(5, 0), 9)),
            position(),
            Offset::Offset(2)
        ));

        assert_eq!(values(&batch.events), ["v"]);
        let kinds: Vec<EventKind> = batch.changes.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [EventKind::Amend]);
        assert_eq!(batch.offsets[&position()], Offset::Offset(2));
    }

    #[test]
    fn a_batch_applies_its_changes_per_key_in_kafka_order() {
        let mut other = stored("other", at(3, 0), 2, false);
        other.customer_id = Uuid::from_u128(2);
        let copies = HashMap::from([
            (
                key(&event(1, "a", "", false)),
                vec![stored("old", at(2, 0), 1, false)],
            ),
            (key(&event(2, "a", "", false)), vec![other]),
        ]);
        let of_customer_2 = |mut change: RawEvent| {
            change.customer_id = CustomerId::from(Uuid::from_u128(2));
            change
        };
        let changes = [
            change(EventKind::Amend, at(5, 0), 9),
            of_customer_2(change(EventKind::Void, at(9, 0), 10)),
            change(EventKind::Amend, at(6, 0), 11),
            of_customer_2(change(EventKind::Amend, at(7, 0), 12)),
            change(EventKind::Amend, at(5, 0), 9),
        ];

        let plan = plan_changes(&changes, copies);

        assert_eq!(rows_of(&plan.new_rows), [("new", 9, 0), ("new", 11, 0)]);
        assert_eq!(
            rows_of(&plan.tombstones),
            [("old", 1, 1), ("other", 2, 1), ("new", 9, 1)]
        );
        assert_eq!(plan.tombstones[1].customer_id, Uuid::from_u128(2));
        let inserts = plan.inserts();
        assert_eq!(inserts.len(), 1);
        assert_eq!(inserts[0].len(), 5);
    }

    #[test]
    fn a_change_skips_the_first_wins_drops() {
        let mut batch = Batch::default();
        batch.push(
            Some(event(1, "a", "v", false)),
            ("raw".to_string(), 0),
            Offset::Offset(1),
        );
        batch.push(
            Some(change(EventKind::Void, at(5, 0), 9)),
            ("raw".to_string(), 0),
            Offset::Offset(2),
        );

        let seen = HashSet::from([key(&batch.events[0])]);
        assert!(drop_seen(dedup_in_batch(batch.events), &seen).is_empty());
        let kinds: Vec<EventKind> = batch.changes.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [EventKind::Void]);
    }

    #[test]
    fn the_kind_travels_on_the_wire_and_defaults_to_event() {
        let amend = change(EventKind::Amend, at(5, 0), 9);
        let json = serde_json::to_value(&amend).unwrap();
        assert_eq!(json["kind"], "amend");
        assert_eq!(serde_json::from_value::<RawEvent>(json).unwrap(), amend);

        let mut legacy = serde_json::to_value(event(1, "a", "v", false)).unwrap();
        legacy.as_object_mut().unwrap().remove("kind");
        assert_eq!(
            serde_json::from_value::<RawEvent>(legacy).unwrap().kind,
            EventKind::Event
        );
    }

    #[test]
    fn keep_last_flag_defaults_to_false_on_the_wire() {
        let mut json = serde_json::to_value(event(1, "a", "v", true)).unwrap();
        json.as_object_mut().unwrap().remove("keep_last");

        assert!(!serde_json::from_value::<RawEvent>(json).unwrap().keep_last);
    }
}
