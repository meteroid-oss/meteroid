use std::collections::HashMap;

use crate::compute::errors::ComputeError;
use crate::domain::{BillableMetric, Period};
use chrono::NaiveDate;
use common_domain::ids::{BillableMetricId, CustomerId, TenantId};
use rust_decimal::Decimal;

#[derive(Debug, Clone)]
pub struct UsageData {
    pub data: Vec<GroupedUsageData>,
    pub period: Period,
}

impl UsageData {
    pub(crate) fn single(&self) -> Result<Decimal, ComputeError> {
        if self.data.len() > 1 {
            return Err(ComputeError::TooManyResults);
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
    ) -> Result<Vec<Metadata>, ComputeError>;

    async fn fetch_usage(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        customer_alias: &Option<String>,
        metric: &BillableMetric,
        period: Period,
    ) -> Result<UsageData, ComputeError>;
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
    ) -> Result<Vec<Metadata>, ComputeError> {
        Ok(vec![])
    }

    async fn fetch_usage(
        &self,
        _tenant_id: TenantId,
        _customer_local_id: CustomerId,
        _customer_alias: &Option<String>,
        metric: &BillableMetric,
        period: Period,
    ) -> Result<UsageData, ComputeError> {
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
