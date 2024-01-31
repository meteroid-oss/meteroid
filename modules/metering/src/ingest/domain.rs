use chrono::NaiveDateTime;
use std::collections::HashMap;

use metering_grpc::meteroid::metering::v1::Event;
use serde::Serialize;

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
