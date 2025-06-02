use std::collections::HashMap;

use crate::StoreResult;
use crate::domain::{BillableMetric, Period};
use crate::errors::StoreError;
use chrono::NaiveDate;
use common_domain::ids::{BillableMetricId, CustomerId, TenantId};
use error_stack::Report;
use rust_decimal::Decimal;

#[derive(Debug, Clone)]
pub struct UsageData {
    pub data: Vec<GroupedUsageData>,
    pub period: Period,
}

impl UsageData {
    pub(crate) fn single(&self) -> StoreResult<Decimal> {
        if self.data.len() > 1 {
            return Err(
                Report::new(StoreError::MeteringServiceError).attach_printable("Too many results")
            );
        }
        Ok(self
            .data
            .first()
            .map(|usage| usage.value)
            .unwrap_or(Decimal::ZERO))
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
}

impl MockUsageClient {
    pub fn noop() -> Self {
        Self {
            data: HashMap::new(),
        }
    }
}
