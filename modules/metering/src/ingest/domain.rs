use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use clickhouse::Row;
use metering_grpc::meteroid::metering::v1::Event;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Default, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RawEvent {
    pub id: String,
    pub code: String,
    pub customer_id: String,
    pub tenant_id: String,
    pub timestamp: NaiveDateTime,
    pub ingested_at: NaiveDateTime,
    pub properties: HashMap<String, String>,
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
            customer_id: event.customer_id,
            tenant_id: event.tenant_id,
            timestamp: Utc.from_utc_datetime(&event.timestamp),
            ingested_at: Utc.from_utc_datetime(&event.ingested_at),
            properties: event.properties.into_iter().collect(),
        }
    }
}

pub struct FailedEvent {
    pub event: Event,
    pub reason: String,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize, Eq, PartialEq, Row)]
/// NOTE: the order of fields must match the order in the `ClickHouse` table
pub struct RawEventRow {
    pub id: String,
    pub code: String,
    pub customer_id: String,
    pub tenant_id: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub timestamp: DateTime<Utc>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub ingested_at: DateTime<Utc>,
    // clickhouse crate doesn't support Map in decoder/encoder
    pub properties: Vec<(String, String)>,
}
