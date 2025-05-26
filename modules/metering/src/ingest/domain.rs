use chrono::{DateTime, NaiveDateTime, Utc};
use clickhouse::Row;
use metering_grpc::meteroid::metering::v1::Event;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Default, Debug, Serialize, Eq, PartialEq)]
pub struct ProcessedEvent {
    pub event_id: String,
    pub event_name: String,
    pub customer_id: String,
    pub tenant_id: String,
    pub event_timestamp: NaiveDateTime,
    pub properties: HashMap<String, String>,
}

impl ProcessedEvent {
    pub fn key(&self) -> String {
        format!("{}:{}", self.tenant_id, self.event_id)
    }
}

pub struct FailedEvent {
    pub event: Event,
    pub reason: String,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize, Eq, PartialEq, Row)]
pub struct ProcessedEventRow {
    pub event_id: String,
    pub event_name: String,
    pub customer_id: String,
    pub tenant_id: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub event_timestamp: DateTime<Utc>,
    // clickhouse crate doesn't support Map in decoder/encoder
    pub properties: Vec<(String, String)>,
}
