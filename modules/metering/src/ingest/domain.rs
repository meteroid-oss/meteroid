use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use clickhouse::Row;
use common_domain::ids::{CustomerId, TenantId};
use metering_grpc::meteroid::metering::v1::Event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RawEvent {
    pub id: String,
    pub code: String,
    pub customer_id: CustomerId,
    pub tenant_id: TenantId,
    pub timestamp: NaiveDateTime,
    pub ingested_at: NaiveDateTime,
    pub properties: HashMap<String, String>,
    /// Kafka-only: the tenant keeps the last copy of an id, so the consumer skips dedup.
    #[serde(default)]
    pub keep_last: bool,
    /// Kafka-only (ADR-0012 §4). An amend carries the new timestamp and properties, and the
    /// `ingested_at` its new row gets.
    #[serde(default)]
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    #[default]
    Event,
    Amend,
    Void,
}

impl RawEvent {
    pub fn key(&self) -> String {
        format!("{}:{}", self.tenant_id, self.id)
    }
}

impl From<RawEvent> for RawEventRow {
    fn from(event: RawEvent) -> Self {
        RawEventRow {
            id: event.id,
            code: event.code,
            customer_id: *event.customer_id,
            tenant_id: *event.tenant_id,
            timestamp: Utc.from_utc_datetime(&event.timestamp),
            ingested_at: Utc.from_utc_datetime(&event.ingested_at),
            properties: event.properties.into_iter().collect(),
            is_tombstone: 0,
        }
    }
}

pub struct FailedEvent {
    pub event: Event,
    pub reason: String,
}

/// Inserts name their columns; reads select these columns explicitly.
#[derive(Clone, Debug, Serialize, Deserialize, Row, PartialEq)]
pub struct RawEventRow {
    pub id: String,
    pub code: String,
    #[serde(with = "clickhouse::serde::uuid")]
    pub customer_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    pub tenant_id: Uuid,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub timestamp: DateTime<Utc>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub ingested_at: DateTime<Utc>,
    /// Sorted, so rows rebuilt from the same data serialize to the same block.
    pub properties: BTreeMap<String, String>,
    /// 1 on the copy that cancels the stored row it duplicates (ADR-0012 §2).
    pub is_tombstone: u8,
}
