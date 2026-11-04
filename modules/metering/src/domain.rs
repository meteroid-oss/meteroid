use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, TenantId};
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use std::collections::HashMap;

pub use metric_core::domain::{Usage, WindowSize};

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

impl MeterAggregation {
    /// Whether values of disjoint event sets sum to the value of their union.
    pub fn is_additive(&self) -> bool {
        matches!(self, MeterAggregation::Sum | MeterAggregation::Count)
    }
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
pub enum SegmentationFilter {
    Independent(Vec<(String, Vec<String>)>),
    Linked {
        dimension1_key: String,
        dimension2_key: String,
        values: HashMap<String, Vec<String>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyFilterOp {
    In,
    NotIn,
}

/// A pre-aggregation WHERE filter on an event property. Applied to the meter's raw-event
/// read ONLY — never to SELECT/GROUP BY — so it restricts which events are aggregated
/// without splitting the rated result (unlike [`SegmentationFilter`], which also groups).
/// `In` keeps events whose property is one of `values`; `NotIn` keeps events that CARRY the
/// property and whose value is none of `values` (presence-required negation).
#[derive(Debug, Clone)]
pub struct PropertyFilter {
    pub property: String,
    pub op: PropertyFilterOp,
    pub values: Vec<String>,
}

/// The events of a subscription among its customer's: those of the segment their timestamp falls
/// in, which its `matching` property has (if any), less those another subscription claims.
/// Applied after deduplication.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventAttribution {
    pub segments: Vec<AttributionSegment>,
}

/// `[from, to)`, unbounded where absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionSegment {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub matching: Option<EventMatch>,
    pub excluded: Vec<EventClaim>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventMatch {
    pub property: String,
    pub value: String,
}

/// Events whose `property` is `value`, timestamped in `[from, to)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventClaim {
    pub property: String,
    pub value: String,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
}

/// Which events a usage read counts. Every query builder takes it, so restricting usage by
/// origin later is one added condition rather than a new query path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OriginFilter {
    #[default]
    All,
}

#[derive(Debug, Clone)]
pub struct QueryMeterParams {
    pub aggregation: MeterAggregation,
    pub tenant_id: TenantId,
    pub code: String,
    pub value_property: Option<String>,
    pub customer_ids: Vec<CustomerId>,
    pub segmentation_filter: Option<SegmentationFilter>,
    /// Restricts to the segments without breaking the result down by them.
    pub merge_segments: bool,
    /// Pre-aggregation property filters (WHERE-only), ANDed together. Empty = no filter,
    /// which yields byte-identical SQL to before this field existed.
    pub filters: Vec<PropertyFilter>,
    pub attribution: Option<EventAttribution>,
    pub origin: OriginFilter,
    /// Breaks the result down by customer even when `customer_ids` is empty.
    pub group_by_customer: bool,
    pub group_by: Vec<String>,
    pub window_size: Option<WindowSize>,
    pub window_time_zone: Option<chrono_tz::Tz>,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
    /// The tenant's `keep_last_events`: the last copy ingested of an id wins.
    pub keep_last_events: bool,
}

impl QueryMeterParams {
    /// Whether the result rows carry the `customer_id` column.
    pub fn groups_by_customer(&self) -> bool {
        self.group_by_customer || !self.customer_ids.is_empty()
    }
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
    pub tenant_id: TenantId,
    pub from: DateTime<Utc>,
    pub to: Option<DateTime<Utc>>,
    pub limit: u32,
    pub offset: u32,
    pub search: Option<String>,
    pub event_codes: Vec<String>,
    pub customer_ids: Vec<CustomerId>,
    /// Pre-aggregation property filters (WHERE-only), ANDed. Lets the events explorer show
    /// only the events a metric would count. Empty = no filter.
    pub filters: Vec<PropertyFilter>,
    pub sort_order: EventSortOrder,
    /// Keyset position for timestamp sort orders; takes precedence over `offset` semantics.
    pub after: Option<EventCursor>,
    pub keep_last_events: bool,
}

#[derive(Debug, Clone)]
pub struct EventCursor {
    pub timestamp_nanos: i64,
    pub id: String,
}

#[derive(Debug)]
pub struct QueryRawEventsResult {
    pub events: Vec<crate::ingest::domain::RawEvent>,
}

#[derive(Debug, Clone)]
pub struct TenantFirstEvent {
    pub tenant_id: TenantId,
    pub first_ingested_at: DateTime<Utc>,
}

/// When one customer of a tenant first sent us an event.
#[derive(Debug, Clone)]
pub struct CustomerFirstEvent {
    pub customer_id: CustomerId,
    pub first_ingested_at: DateTime<Utc>,
}

/// A tenant to scan, and the instant to scan from. `None` = no bound.
#[derive(Debug, Clone)]
pub struct TenantEventScan {
    pub tenant_id: TenantId,
    pub from: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct UsageActivityParams {
    pub since: DateTime<Utc>,
    pub after: Option<ActivityPosition>,
    pub shard_index: u32,
    pub shard_count: u32,
    pub limit: u32,
}

/// Where a page of activity ended: results resume strictly after it, in result order.
#[derive(Debug, Clone)]
pub struct ActivityPosition {
    pub last_seen_at: DateTime<Utc>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
}

#[derive(Debug, Clone)]
pub struct CustomerActivity {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub last_seen_at: DateTime<Utc>,
    pub min_event_at: DateTime<Utc>,
}

/// One discovered raw-event property: the key, how many distinct values it takes and a
/// sample of those values.
#[derive(Debug, Clone)]
pub struct DiscoveredProperty {
    pub key: String,
    pub cardinality: u64,
    pub sample_values: Vec<String>,
    /// Fraction of non-empty values parseable as a number, in [0, 1]. Best-effort.
    pub numeric_ratio: f64,
}

#[derive(Debug, Clone)]
pub struct PropertyValueCount {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct EventCodeCount {
    pub code: String,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct PropertyNumericStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub sum: f64,
    pub p50: f64,
    pub p95: f64,
}

#[derive(Debug, Clone)]
pub struct PropertyProfile {
    pub key: String,
    pub event_count: u64,
    pub coverage_ratio: f64,
    pub cardinality: u64,
    pub numeric_ratio: f64,
    pub numeric: Option<PropertyNumericStats>,
    pub top_values: Vec<PropertyValueCount>,
}

/// A code's property profiles plus the coverage denominator.
#[derive(Debug, Clone)]
pub struct EventPropertiesProfile {
    pub total_events: u64,
    pub properties: Vec<PropertyProfile>,
}
