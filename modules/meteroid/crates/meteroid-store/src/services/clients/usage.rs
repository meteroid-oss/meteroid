use crate::StoreResult;
use crate::domain::{BillableMetric, MetricDefinition, UsagePeriod};
use crate::errors::StoreError;
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{BaseId, BillableMetricId, CustomerId, TenantId};
use error_stack::bail;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// A tenant to scan for its first metering event, and the instant to scan from.
/// Sandbox tenants start at their first user-created API key — the seeder
/// ingests without one, so anything earlier is seeded. `None` = no bound.
#[derive(Debug, Clone, PartialEq, Eq, diesel::QueryableByName)]
pub struct TenantEventScan {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub tenant_id: TenantId,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamp>)]
    pub from: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
pub struct UsageData {
    pub data: Vec<GroupedUsageData>,
    pub period: UsagePeriod,
}

#[derive(Debug, Clone)]
pub struct GroupedUsageData {
    pub value: Decimal,
    pub dimensions: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct WindowedUsageData {
    pub data: Vec<WindowedUsagePoint>,
    pub period: UsagePeriod,
}

#[derive(Debug, Clone)]
pub struct WindowedUsagePoint {
    pub window_start: NaiveDate,
    pub window_end: NaiveDate,
    pub value: Decimal,
    pub dimensions: HashMap<String, String>,
}

/// A discovered raw-event property: its key, value cardinality and a small sample.
#[derive(Debug, Clone)]
pub struct DiscoveredProperty {
    pub key: String,
    pub cardinality: u64,
    pub sample_values: Vec<String>,
    /// Fraction of non-empty values parseable as a number, in [0, 1] — number vs.
    /// text hint for the UI. Best-effort.
    pub numeric_ratio: f64,
}

/// One distinct value of a property and how many events carried it.
#[derive(Debug, Clone)]
pub struct PropertyValueCount {
    pub value: String,
    pub count: u64,
}

/// One distinct event code and how many events (of a tenant, in range) carried it.
#[derive(Debug, Clone)]
pub struct EventCodeCount {
    pub code: String,
    pub count: u64,
}

/// Distribution stats for a property whose values are (mostly) numeric.
#[derive(Debug, Clone)]
pub struct PropertyNumericStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub sum: f64,
    pub p50: f64,
    pub p95: f64,
}

/// Full statistical profile of one raw-event property over a time range.
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

/// Result of profiling a code's properties: the coverage denominator plus each key.
#[derive(Debug, Clone)]
pub struct EventPropertiesProfile {
    pub total_events: u64,
    pub properties: Vec<PropertyProfile>,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventSearchOptions {
    pub from: Option<prost_types::Timestamp>,
    pub to: Option<prost_types::Timestamp>,
    pub limit: u32,
    pub offset: u32,
    pub search: Option<String>,
    pub event_codes: Vec<String>,
    pub customer_ids: Vec<String>,
    /// Pre-aggregation property filters — list only events a metric would count. Carries the
    /// metering proto type directly (like `EventSearchResult::events`).
    pub filters: Vec<metering_grpc::meteroid::metering::v1::PropertyFilter>,
    pub sort_order: i32,
}

#[derive(Debug, Clone)]
pub struct EventSearchResult {
    pub events: Vec<metering_grpc::meteroid::metering::v1::Event>,
}

#[derive(Debug, Clone)]
pub struct IngestEventsRequest {
    pub events: Vec<metering_grpc::meteroid::metering::v1::Event>,
    pub allow_backfilling: bool,
    pub fail_on_error: bool,
    pub keep_last_events: bool,
}

#[derive(Debug, Clone)]
pub struct IngestEventsFailure {
    pub event_id: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct IngestEventsResult {
    pub failures: Vec<IngestEventsFailure>,
}

/// Window size for a tenant-wide usage time series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantUsageGrain {
    Hour,
    Day,
}

/// One window of a usage time series, at full datetime fidelity.
#[derive(Debug, Clone)]
pub struct UsageSeriesPoint {
    pub window_start: NaiveDateTime,
    pub value: Decimal,
    pub dimensions: HashMap<String, String>,
}

/// A position in a customer's event stream: events order by (timestamp, id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricEventCursor {
    pub timestamp_nanos: i64,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct MetricEvent {
    pub id: String,
    pub timestamp: NaiveDateTime,
    pub properties: HashMap<String, String>,
}

/// A customer that received events, with the newest activity time seen and the oldest event time
/// among them.
#[derive(Debug, Clone)]
pub struct CustomerActivity {
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub min_event_at: chrono::DateTime<chrono::Utc>,
}

/// Where a page of activity ended: the next page resumes strictly after it.
#[derive(Debug, Clone, Copy)]
pub struct ActivityPosition {
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
}

impl From<&CustomerActivity> for ActivityPosition {
    fn from(activity: &CustomerActivity) -> Self {
        Self {
            last_seen_at: activity.last_seen_at,
            tenant_id: activity.tenant_id,
            customer_id: activity.customer_id,
        }
    }
}

/// One point of a tenant-wide usage time series. Unlike [`WindowedUsagePoint`]
/// (day-only `NaiveDate`), this keeps full datetime bounds so an hourly grain is
/// not silently truncated.
#[derive(Debug, Clone)]
pub struct TenantUsagePoint {
    pub window_start: NaiveDateTime,
    pub window_end: NaiveDateTime,
    pub value: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricUsageBreakdown {
    None,
    Customer,
    Property(String),
}

/// A metric's usage over a period, as billing reads it: its filters, its segmentation matrix as
/// a filter (segments merged), and values in the billed unit before rounding.
#[derive(Debug, Clone)]
pub struct MetricUsageQuery<'a> {
    pub metric: &'a MetricDefinition,
    /// Empty = every customer of the tenant.
    pub customer_ids: &'a [CustomerId],
    pub breakdown: MetricUsageBreakdown,
    /// `None` = one window over the whole period.
    pub grain: Option<TenantUsageGrain>,
    /// Keeps the largest `n` series of the breakdown; see `MetricUsagePoint::other`.
    pub top_n: Option<u32>,
    pub period: UsagePeriod,
}

#[derive(Debug, Clone)]
pub struct MetricUsagePoint {
    pub window_start: NaiveDateTime,
    pub window_end: NaiveDateTime,
    pub value: Decimal,
    pub dimensions: HashMap<String, String>,
    /// Set under a customer breakdown, except on the `other` point.
    pub customer_id: Option<CustomerId>,
    /// The series beyond the top N, summed per window. Only additive aggregations (sum, count)
    /// have one; the others drop those series.
    pub other: bool,
}

#[async_trait::async_trait]
pub trait UsageClient: Send + Sync {
    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData>;

    /// Usage broken down by `group_by`, on top of the dimensions the metric already has.
    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData>;

    /// [`Self::fetch_usage_grouped_by`] for the one group `value`, which a backend can ask for
    /// instead of reading every group. The default reads them all.
    async fn fetch_usage_of_group(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        group_by: &str,
        _value: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.fetch_usage_grouped_by(tenant_id, customer_id, metric, group_by, period)
            .await
    }

    async fn fetch_total_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal>;

    async fn fetch_windowed_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData>;

    /// The metric's usage over `period` in hourly or daily windows (UTC), split by the metric's own
    /// dimensions and `group_by`; only windows with usage are returned. Default impl is
    /// unsupported; the metering-backed client overrides it.
    async fn fetch_usage_series(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        _group_by: Option<&str>,
        _grain: TenantUsageGrain,
        _period: UsagePeriod,
    ) -> StoreResult<Vec<UsageSeriesPoint>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "fetch_usage_series is not supported by this usage client".to_string(),
        )))
    }

    /// Up to `limit` events of the metric's code (with its property filters) for the customer over
    /// `period`, oldest first, resuming strictly after `after`. Default impl is unsupported; the
    /// metering-backed client overrides it.
    async fn list_metric_events(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        _period: UsagePeriod,
        _after: Option<MetricEventCursor>,
        _limit: u32,
    ) -> StoreResult<Vec<MetricEvent>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "list_metric_events is not supported by this usage client".to_string(),
        )))
    }

    /// Fetch aggregated usage for a metric over a period.
    /// If `customer_id` is Some, scoped to that customer; if None, cross-customer aggregation.
    async fn fetch_usage_summary(
        &self,
        tenant_id: &TenantId,
        customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData>;

    async fn search_events(
        &self,
        tenant_id: &TenantId,
        options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult>;
    async fn ingest_events(
        &self,
        tenant_id: &TenantId,
        request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult>;

    /// Customers that received events after `since`, oldest activity first, resuming strictly
    /// after `after`. Default impl is unsupported; the metering-backed client overrides it.
    async fn poll_usage_activity(
        &self,
        _since: chrono::DateTime<chrono::Utc>,
        _after: Option<ActivityPosition>,
        _limit: u32,
    ) -> StoreResult<Vec<CustomerActivity>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "poll_usage_activity is not supported by this usage client".to_string(),
        )))
    }

    /// Raw events of `metric`'s event code for the customer over `period`, whatever the metric
    /// aggregates. Default impl is unsupported; the metering-backed client overrides it.
    async fn count_events(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        _period: UsagePeriod,
    ) -> StoreResult<u64> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "count_events is not supported by this usage client".to_string(),
        )))
    }

    /// Default impl is unsupported; the metering-backed client overrides it.
    async fn query_metric_usage(
        &self,
        _tenant_id: &TenantId,
        _query: MetricUsageQuery<'_>,
    ) -> StoreResult<Vec<MetricUsagePoint>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "query_metric_usage is not supported by this usage client".to_string(),
        )))
    }

    /// Discover the distinct raw-event property keys (with cardinality + samples)
    /// for a tenant's events of one `event_code`, to drive a dynamic dimension
    /// picker. Default impl is unsupported; the metering-backed client overrides it.
    async fn discover_properties(
        &self,
        _tenant_id: &TenantId,
        _event_code: &str,
        _lookback_days: u32,
        _limit: u32,
    ) -> StoreResult<Vec<DiscoveredProperty>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "discover_properties is not supported by this usage client".to_string(),
        )))
    }

    /// Distinct event codes for a tenant (most frequent first, optional
    /// case-insensitive substring `search`), to drive an event-code autocomplete.
    /// Default impl is unsupported; the metering-backed client overrides it.
    async fn discover_event_codes(
        &self,
        _tenant_id: &TenantId,
        _search: &str,
        _limit: u32,
    ) -> StoreResult<Vec<EventCodeCount>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "discover_event_codes is not supported by this usage client".to_string(),
        )))
    }

    /// Top observed values of one property of one event code (most frequent first,
    /// optional case-insensitive substring `search`), to drive a value autocomplete
    /// in filters / dimensions. Default impl is unsupported; the metering-backed
    /// client overrides it.
    async fn discover_property_values(
        &self,
        _tenant_id: &TenantId,
        _event_code: &str,
        _property_key: &str,
        _search: &str,
        _limit: u32,
    ) -> StoreResult<Vec<PropertyValueCount>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "discover_property_values is not supported by this usage client".to_string(),
        )))
    }

    /// Full statistical profile of a code's raw-event properties over a time range
    /// (coverage, cardinality, numeric distribution, top values). `from_ts`/`to_ts`
    /// are epoch seconds (0 => server default trailing window). Default impl is
    /// unsupported; the metering-backed client overrides it.
    async fn profile_event_properties(
        &self,
        _tenant_id: &TenantId,
        _event_code: &str,
        _from_ts: i64,
        _to_ts: i64,
        _top_values: u32,
        _limit: u32,
    ) -> StoreResult<EventPropertiesProfile> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "profile_event_properties is not supported by this usage client".to_string(),
        )))
    }

    /// Tenant-wide usage time series for a metric over `period`, windowed by
    /// `grain`, aggregated across all customers (empty customer scope). Scope is
    /// enforced by `tenant_id`; the metric's own `code`/`aggregation_type`/
    /// `aggregation_key` drive the raw-events QueryMeter. Default impl is
    /// unsupported; the metering-backed client overrides it.
    async fn fetch_tenant_usage(
        &self,
        _tenant_id: &TenantId,
        _metric: &BillableMetric,
        _period: UsagePeriod,
        _grain: TenantUsageGrain,
    ) -> StoreResult<Vec<TenantUsagePoint>> {
        Err(error_stack::Report::new(StoreError::InvalidArgument(
            "fetch_tenant_usage is not supported by this usage client".to_string(),
        )))
    }

    /// Purge the given deleted tenants' metering raw events, batched. Driven by the weekly
    /// cleanup worker. Default impl is a no-op; the metering-backed client overrides it.
    async fn purge_tenants(&self, _tenant_ids: &[TenantId]) -> StoreResult<()> {
        Ok(())
    }

    /// First ingested raw event per tenant, each no earlier than its own cutoff
    /// (empty filter = all tenants, unbounded). Backs dogfooding activation
    /// milestones. Default impl is empty; the metering-backed client overrides it.
    async fn list_tenant_first_events(
        &self,
        _tenants: &[TenantEventScan],
    ) -> StoreResult<Vec<TenantFirstIngestedEvent>> {
        Ok(vec![])
    }

    /// First ingested event for each of the named customers of one tenant.
    /// Default impl is empty; the metering-backed client overrides it.
    async fn list_customer_first_events(
        &self,
        _tenant_id: &TenantId,
        _customer_ids: &[CustomerId],
    ) -> StoreResult<Vec<CustomerFirstIngestedEvent>> {
        Ok(vec![])
    }
}

/// When one customer of a tenant first sent an event.
#[derive(Debug, Clone)]
pub struct CustomerFirstIngestedEvent {
    pub customer_id: CustomerId,
    pub first_ingested_at: NaiveDateTime,
}

/// When the first raw usage event of a tenant was ingested.
#[derive(Debug, Clone)]
pub struct TenantFirstIngestedEvent {
    pub tenant_id: TenantId,
    pub first_ingested_at: NaiveDateTime,
}

#[derive(Eq, Hash, PartialEq)]
pub struct MockUsageDataParams {
    pub metric_id: BillableMetricId,
    pub period_start: NaiveDateTime,
    pub period_end: NaiveDateTime,
}

pub struct MockUsageClient {
    pub data: HashMap<MockUsageDataParams, UsageData>,
}

#[async_trait::async_trait]
impl UsageClient for MockUsageClient {
    async fn fetch_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        // Match the production client: an empty window is an invalid query, not zero usage.
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }
        let params = MockUsageDataParams {
            metric_id: metric.id,
            period_start: period.start,
            period_end: period.end,
        };
        let usage_data = self
            .data
            .get(&params)
            .cloned()
            .unwrap_or_else(|| UsageData {
                data: vec![],
                period: period.clone(),
            });
        Ok(usage_data)
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        _group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.fetch_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_total_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        let params = MockUsageDataParams {
            metric_id: metric.id,
            period_start: period.start,
            period_end: period.end,
        };
        Ok(self
            .data
            .get(&params)
            .map(|d| d.data.iter().map(|g| g.value).sum())
            .unwrap_or(Decimal::ZERO))
    }

    async fn fetch_windowed_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        _metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        Ok(WindowedUsageData {
            data: vec![],
            period,
        })
    }

    async fn fetch_usage_summary(
        &self,
        _tenant_id: &TenantId,
        _customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        let params = MockUsageDataParams {
            metric_id: metric.id,
            period_start: period.start,
            period_end: period.end,
        };
        let usage_data = self
            .data
            .get(&params)
            .cloned()
            .unwrap_or_else(|| UsageData {
                data: vec![],
                period: period.clone(),
            });
        Ok(usage_data)
    }

    async fn search_events(
        &self,
        _tenant_id: &TenantId,
        _options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        bail!(StoreError::InvalidArgument(
            "Mock client does not support event search".to_string()
        ));
    }

    async fn ingest_events(
        &self,
        _tenant_id: &TenantId,
        _request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        bail!(StoreError::InvalidArgument(
            "Mock client does not support event ingestion".to_string()
        ));
    }
}

impl MockUsageClient {
    pub fn noop() -> Self {
        Self {
            data: HashMap::new(),
        }
    }
}

/// A usage mock whose data can be mutated after construction; clones share one store, so a test's
/// handle and the one inside the `Store` see the same mutations. Lets a test model usage imported
/// *late* (during the grace period, after the draft exists), which [`MockUsageClient`] cannot.
#[derive(Clone, Default)]
pub struct MutableMockUsageClient {
    data: std::sync::Arc<std::sync::Mutex<HashMap<MockUsageDataParams, UsageData>>>,
    /// Usage returned for a metric on any *non-empty* query window (`end > start`), regardless of
    /// the exact bounds. Models a real metering backend (a zero-width window returns nothing) for
    /// tests that cannot predict the exact window — e.g. an immediate cancel whose upper bound is
    /// the non-deterministic wall-clock instant. Consulted only when the exact `data` map misses.
    nonempty_window: std::sync::Arc<std::sync::Mutex<HashMap<BillableMetricId, UsageData>>>,
    /// Daily points of a metric; a windowed query returns those starting inside its window.
    windowed: std::sync::Arc<std::sync::Mutex<HashMap<BillableMetricId, Vec<WindowedUsagePoint>>>>,
    activity: std::sync::Arc<std::sync::Mutex<Vec<CustomerActivity>>>,
    activity_pages: std::sync::Arc<std::sync::Mutex<Vec<Vec<CustomerActivity>>>>,
}

impl MutableMockUsageClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or replace) the usage returned for a given metric + period.
    pub fn set_usage(&self, params: MockUsageDataParams, data: UsageData) {
        self.data.lock().unwrap().insert(params, data);
    }

    /// Return `data` for `metric_id` on any query window with `end > start`, and nothing on an
    /// empty window. See [`Self::nonempty_window`].
    pub fn set_usage_for_any_nonempty_window(&self, metric_id: BillableMetricId, data: UsageData) {
        self.nonempty_window.lock().unwrap().insert(metric_id, data);
    }

    /// Set (or replace) the daily points returned by windowed queries of `metric_id`.
    pub fn set_windowed_usage(&self, metric_id: BillableMetricId, points: Vec<WindowedUsagePoint>) {
        self.windowed.lock().unwrap().insert(metric_id, points);
    }

    /// The activity the backend reports, polled with its ordering and keyset paging.
    pub fn set_activity(&self, activity: Vec<CustomerActivity>) {
        *self.activity.lock().unwrap() = activity;
    }

    /// Every page `poll_usage_activity` has returned, in call order.
    pub fn activity_pages(&self) -> Vec<Vec<CustomerActivity>> {
        self.activity_pages.lock().unwrap().clone()
    }

    fn lookup(&self, params: &MockUsageDataParams, period: &UsagePeriod) -> UsageData {
        if let Some(data) = self.data.lock().unwrap().get(params).cloned() {
            return data;
        }
        if period.end > period.start
            && let Some(data) = self
                .nonempty_window
                .lock()
                .unwrap()
                .get(&params.metric_id)
                .cloned()
        {
            return data;
        }
        UsageData {
            data: vec![],
            period: period.clone(),
        }
    }
}

#[async_trait::async_trait]
impl UsageClient for MutableMockUsageClient {
    async fn poll_usage_activity(
        &self,
        since: chrono::DateTime<chrono::Utc>,
        after: Option<ActivityPosition>,
        limit: u32,
    ) -> StoreResult<Vec<CustomerActivity>> {
        let key = |a: &CustomerActivity| {
            (
                a.last_seen_at,
                a.tenant_id.as_uuid(),
                a.customer_id.as_uuid(),
            )
        };
        let after = after.map(|p| {
            (
                p.last_seen_at,
                p.tenant_id.as_uuid(),
                p.customer_id.as_uuid(),
            )
        });
        let mut page = self
            .activity
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.last_seen_at > since && after.is_none_or(|after| key(a) > after))
            .cloned()
            .collect::<Vec<_>>();
        page.sort_by_key(key);
        page.truncate(limit as usize);
        self.activity_pages.lock().unwrap().push(page.clone());
        Ok(page)
    }

    async fn fetch_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        // Match the production client: an empty window is an invalid query, not zero usage.
        if period.start >= period.end {
            bail!(StoreError::InvalidArgument("invalid period".to_string()));
        }
        let params = MockUsageDataParams {
            metric_id: metric.id,
            period_start: period.start,
            period_end: period.end,
        };
        Ok(self.lookup(&params, &period))
    }

    async fn fetch_usage_grouped_by(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        _group_by: &str,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        self.fetch_usage(tenant_id, customer_id, metric, period)
            .await
    }

    async fn fetch_total_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<Decimal> {
        let params = MockUsageDataParams {
            metric_id: metric.id,
            period_start: period.start,
            period_end: period.end,
        };
        Ok(self
            .lookup(&params, &period)
            .data
            .iter()
            .map(|g| g.value)
            .sum())
    }

    async fn fetch_windowed_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<WindowedUsageData> {
        let (start, end) = (period.start.date(), period.end.date());
        let data = self
            .windowed
            .lock()
            .unwrap()
            .get(&metric.id)
            .map(|points| {
                points
                    .iter()
                    .filter(|p| p.window_start >= start && p.window_start < end)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Ok(WindowedUsageData { data, period })
    }

    async fn fetch_usage_summary(
        &self,
        _tenant_id: &TenantId,
        _customer_id: Option<&CustomerId>,
        metric: &BillableMetric,
        period: UsagePeriod,
    ) -> StoreResult<UsageData> {
        let params = MockUsageDataParams {
            metric_id: metric.id,
            period_start: period.start,
            period_end: period.end,
        };
        Ok(self.lookup(&params, &period))
    }

    async fn search_events(
        &self,
        _tenant_id: &TenantId,
        _options: EventSearchOptions,
    ) -> StoreResult<EventSearchResult> {
        bail!(StoreError::InvalidArgument(
            "Mock client does not support event search".to_string()
        ));
    }

    async fn ingest_events(
        &self,
        _tenant_id: &TenantId,
        _request: IngestEventsRequest,
    ) -> StoreResult<IngestEventsResult> {
        bail!(StoreError::InvalidArgument(
            "Mock client does not support event ingestion".to_string()
        ));
    }
}
