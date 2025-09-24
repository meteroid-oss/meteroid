use chrono::{DateTime, Utc};
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum MeterAggregation {
    Sum,
    Avg,
    Min,
    Max,
    Count,
    Latest,
    CountDistinct,
}

impl From<AggregationType> for MeterAggregation {
    fn from(value: AggregationType) -> Self {
        match value {
            AggregationType::Sum => MeterAggregation::Sum,
            AggregationType::Mean => MeterAggregation::Avg,
            AggregationType::Min => MeterAggregation::Min,
            AggregationType::Max => MeterAggregation::Max,
            AggregationType::Count => MeterAggregation::Count,
            AggregationType::Latest => MeterAggregation::Latest,
            AggregationType::CountDistinct => MeterAggregation::CountDistinct,
        }
    }
}

#[derive(Debug, Clone)]
pub enum WindowSize {
    Minute,
    Hour,
    Day,
}

#[derive(Debug)]
pub struct Meter {
    pub aggregation: MeterAggregation,
    pub namespace: String,
    pub id: String,
    pub code: String,
    pub value_property: Option<String>,
    pub group_by: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct QueryMeterParams {
    pub aggregation: MeterAggregation,
    pub namespace: String,
    pub meter_slug: String,
    pub code: String,
    pub customer_ids: Vec<String>,
    pub filter_group_by: HashMap<String, Vec<String>>,
    pub group_by: Vec<String>,
    pub window_size: Option<WindowSize>,
    pub window_time_zone: Option<String>,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct Usage {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub value: f64,
    pub customer_id: String,
    pub group_by: HashMap<String, Option<String>>,
}

#[derive(Debug, Clone)]
pub enum EventSortOrder {
    TimestampDesc,
    TimestampAsc,
    IngestedDesc,
    IngestedAsc,
}

#[derive(Debug, Clone)]
pub struct QueryRawEventsParams {
    pub tenant_id: String,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
    pub limit: u32,
    pub offset: u32,
    pub search: Option<String>,
    pub event_codes: Vec<String>,
    pub customer_ids: Vec<String>,
    pub sort_order: EventSortOrder,
}

#[derive(Debug)]
pub struct QueryRawEventsResult {
    pub events: Vec<crate::ingest::domain::RawEvent>,
}
