use crate::StoreResult;
use crate::domain::{BillableMetric, Period};
use crate::errors::StoreError;
use chrono::NaiveDate;
use common_domain::ids::{BillableMetricId, CustomerId, TenantId};
use error_stack::{Report, bail};
use rust_decimal::Decimal;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct UsageData {
    pub data: Vec<GroupedUsageData>,
    pub period: Period,
}

impl UsageData {
    pub(crate) fn single(&self) -> StoreResult<Decimal> {
        if self.data.len() > 1 {
            return Err(Report::new(StoreError::MeteringServiceError).attach("Too many results"));
        }
        Ok(self.data.first().map_or(Decimal::ZERO, |usage| usage.value))
    }
}

#[derive(Debug, Clone)]
pub struct GroupedUsageData {
    pub value: Decimal,
    pub dimensions: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct CsvIngestionOptions {
    pub delimiter: char,
    pub allow_backfilling: bool,
    pub fail_on_error: bool,
}

#[derive(Debug, Clone)]
pub struct CsvIngestionFailure {
    pub row_number: i32,
    pub event_id: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct CsvIngestionResult {
    pub total_rows: i32,
    pub successful_events: i32,
    pub failures: Vec<CsvIngestionFailure>,
}

#[derive(Debug, Clone)]
pub struct EventSearchOptions {
    pub from: Option<prost_types::Timestamp>,
    pub to: Option<prost_types::Timestamp>,
    pub limit: u32,
    pub offset: u32,
    pub search: Option<String>,
    pub event_codes: Vec<String>,
    pub customer_ids: Vec<String>,
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

#[async_trait::async_trait]
pub trait UsageClient: Send + Sync {
    async fn register_meter(
        &self,
        tenant_id: TenantId,
        metric: &BillableMetric,
    ) -> StoreResult<Vec<Metadata>>;

    async fn fetch_usage(
        &self,
        tenant_id: &TenantId,
        customer_id: &CustomerId,
        metric: &BillableMetric,
        period: Period,
    ) -> StoreResult<UsageData>;
    async fn ingest_events_from_csv(
        &self,
        tenant_id: &TenantId,
        file_data: &[u8],
        options: CsvIngestionOptions,
    ) -> StoreResult<CsvIngestionResult>;
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
}

#[derive(Eq, Hash, PartialEq)]
pub struct MockUsageDataParams {
    metric_id: BillableMetricId,
    invoice_date: NaiveDate,
}

pub struct MockUsageClient {
    pub data: HashMap<MockUsageDataParams, UsageData>,
}

#[async_trait::async_trait]
impl UsageClient for MockUsageClient {
    async fn register_meter(
        &self,
        _tenant_id: TenantId,
        _metric: &BillableMetric,
    ) -> StoreResult<Vec<Metadata>> {
        Ok(vec![])
    }

    async fn fetch_usage(
        &self,
        _tenant_id: &TenantId,
        _customer_id: &CustomerId,
        metric: &BillableMetric,
        period: Period,
    ) -> StoreResult<UsageData> {
        let params = MockUsageDataParams {
            metric_id: metric.id,
            invoice_date: period.end,
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

    async fn ingest_events_from_csv(
        &self,
        _tenant_id: &TenantId,
        _file_data: &[u8],
        _options: CsvIngestionOptions,
    ) -> StoreResult<CsvIngestionResult> {
        bail!(StoreError::InvalidArgument(
            "Mock client does not support CSV ingestion".to_string()
        ));
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
