use std::collections::HashMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::compute::errors::ComputeError;
use crate::domain::{BillableMetric, Period};

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
            .map(|usage| usage.value.clone())
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
        tenant_id: &Uuid,
        metric: &BillableMetric,
    ) -> Result<Vec<Metadata>, ComputeError>;

    async fn fetch_usage(
        &self,
        tenant_id: &Uuid,
        customer_id: &Uuid,
        customer_external_id: &Option<String>,
        metric: &BillableMetric,
        period: Period,
    ) -> Result<UsageData, ComputeError>;
}

#[derive(Eq, Hash, PartialEq)]
pub struct MockUsageDataParams {
    metric_id: Uuid,
    invoice_date: NaiveDate,
}

pub struct MockUsageClient {
    pub data: HashMap<MockUsageDataParams, UsageData>,
}

#[async_trait::async_trait]
impl UsageClient for MockUsageClient {
    async fn register_meter(
        &self,
        _tenant_id: &Uuid,
        _metric: &BillableMetric,
    ) -> Result<Vec<Metadata>, ComputeError> {
        Ok(vec![])
    }

    async fn fetch_usage(
        &self,
        _tenant_id: &Uuid,
        _customer_id: &Uuid,
        _customer_external_id: &Option<String>,
        metric: &BillableMetric,
        period: Period,
    ) -> Result<UsageData, ComputeError> {
        let params = MockUsageDataParams {
            metric_id: metric.id.clone(),
            invoice_date: period.end.clone(),
        };
        let usage_data = self
            .data
            .get(&params)
            .map(|data| data.clone())
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
