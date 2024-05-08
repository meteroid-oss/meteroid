use std::collections::HashMap;

use chrono::{NaiveDate, Timelike};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::compute::errors::ComputeError;
use crate::compute::Period;
use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{
    QueryMeterRequest, QueryMeterResponse, ResourceIdentifier,
};
use meteroid_store::domain;
use meteroid_store::domain::BillableMetric;

use crate::models::UsageDetails;

#[derive(Debug, Clone)]
pub struct UsageData {
    pub total_usage: Decimal,
    pub usage_details: Vec<UsageDetails>,
    pub period: Period,
}

#[derive(Clone, Debug)]
pub struct MeteringUsageClient {
    metering_client: UsageQueryServiceClient<LayeredClientService>,
}

impl MeteringUsageClient {
    pub fn new(metering_client: UsageQueryServiceClient<LayeredClientService>) -> Self {
        Self {
            metering_client: metering_client.clone(),
        }
    }
}

#[async_trait::async_trait]
pub trait UsageClient {
    async fn fetch_usage(
        &self,
        tenant_id: &Uuid,
        customer_id: &Uuid,
        customer_external_id: &Option<String>,
        metric: &BillableMetric,
        period: Period,
    ) -> Result<UsageData, ComputeError>;
}

#[async_trait::async_trait]
impl UsageClient for MeteringUsageClient {
    async fn fetch_usage(
        &self,
        tenant_id: &Uuid,
        customer_id: &Uuid,
        customer_external_id: &Option<String>,
        metric: &BillableMetric,
        period: Period,
    ) -> Result<UsageData, ComputeError> {
        if period.start >= period.end {
            return Err(ComputeError::InvalidPeriod);
        }

        let aggregation_type = match metric.aggregation_type {
            domain::enums::BillingMetricAggregateEnum::Count => AggregationType::Count,
            domain::enums::BillingMetricAggregateEnum::Latest => AggregationType::Latest,
            domain::enums::BillingMetricAggregateEnum::Max => AggregationType::Max,
            domain::enums::BillingMetricAggregateEnum::Min => AggregationType::Min,
            domain::enums::BillingMetricAggregateEnum::Mean => AggregationType::Mean,
            domain::enums::BillingMetricAggregateEnum::Sum => AggregationType::Sum,
            domain::enums::BillingMetricAggregateEnum::CountDistinct => {
                AggregationType::CountDistinct
            }
        } as i32;

        let request = QueryMeterRequest {
            tenant_id: tenant_id.to_string(),
            meter_slug: metric.id.to_string(),
            meter_aggregation_type: aggregation_type,
            customers: vec![ResourceIdentifier {
                meteroid_id: customer_id.to_string(),
                external_id: customer_external_id
                    .clone()
                    .unwrap_or(customer_id.to_string()), // TODO make mandatory in db, or optional in metering
            }],
            from: Some(date_to_timestamp(period.start)),
            to: Some(date_to_timestamp(period.end)), // exclusive TODO check
            // not used here, defaults to customer_id
            group_by_properties: vec![],
            // the segmentation dimensions TODO
            filter_properties: vec![],
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
        };

        let mut metering_client_mut = self.metering_client.clone();
        let usage_data: QueryMeterResponse = metering_client_mut
            .query_meter(request)
            .await
            .map_err(|_status| ComputeError::MeteringGrpcError)?
            .into_inner();

        //TODO check that length is 1. Alternatively, do a purpose-built query
        let total_usage: Decimal = usage_data
            .usage
            .first()
            .and_then(|u| u.value.as_ref())
            .and_then(|u| u.clone().try_into().ok())
            .unwrap_or_else(|| Decimal::ZERO);

        Ok(UsageData {
            total_usage,
            usage_details: vec![], // we no longer have the day by day unless we decide to fetch separately. We could do that on demand + on finalize
            period,
        })
    }
}

fn date_to_timestamp(dt: NaiveDate) -> prost_types::Timestamp {
    let dt_at_start_of_day = dt.and_hms_opt(0, 0, 0).unwrap();
    prost_types::Timestamp {
        seconds: dt_at_start_of_day.and_utc().timestamp(),
        nanos: dt_at_start_of_day.nanosecond() as i32,
    }
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
                total_usage: Decimal::ZERO,
                usage_details: vec![],
                period: period.clone(),
            });
        Ok(usage_data)
    }
}
