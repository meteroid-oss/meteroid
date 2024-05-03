use crate::compute::fees::shared::period_from_cadence;
use crate::compute::SubscriptionDetails;
use crate::models::{InvoiceLinePeriod, UsageDetails};
use anyhow::anyhow;
use chrono::{NaiveDate, Timelike};
use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{
    QueryMeterRequest, QueryMeterResponse, ResourceIdentifier,
};
use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation::AggregationType as ApiAggregationType;
use meteroid_grpc::meteroid::api::components::v1::fee::BillingType;
use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;

use meteroid_grpc::meteroid::api::billablemetrics::v1::BillableMetric;
use rust_decimal::Decimal;

#[derive(Debug)]
pub struct UsageData {
    pub total_usage: Decimal,
    pub usage_details: Vec<UsageDetails>,
    pub period: InvoiceLinePeriod,
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
        metric: &BillableMetric,
        subscription: &SubscriptionDetails,
    ) -> anyhow::Result<UsageData>;
}

#[async_trait::async_trait]
impl UsageClient for MeteringUsageClient {
    async fn fetch_usage(
        &self,
        metric: &BillableMetric,
        subscription: &SubscriptionDetails,
    ) -> anyhow::Result<UsageData> {
        // TODO case when period is partial (ex: prorated first period, or invoiced early)
        let usage_period =
            period_from_cadence(&BillingPeriod::Monthly, BillingType::Arrear, subscription)?;

        if usage_period.from >= usage_period.to {
            return Err(anyhow!("Invalid usage period. From date is after To date"));
        }

        let aggregation = metric
            .aggregation
            .as_ref()
            .ok_or(anyhow!("No aggregation in metric"))?;

        let aggregation_type = match aggregation.aggregation_type() {
            ApiAggregationType::Count => AggregationType::Count,
            ApiAggregationType::Latest => AggregationType::Latest,
            ApiAggregationType::Max => AggregationType::Max,
            ApiAggregationType::Min => AggregationType::Min,
            ApiAggregationType::Mean => AggregationType::Mean,
            ApiAggregationType::Sum => AggregationType::Sum,
            ApiAggregationType::CountDistinct => AggregationType::CountDistinct,
        } as i32;

        let request = QueryMeterRequest {
            tenant_id: subscription.tenant_id.to_string(),
            meter_slug: metric.id.to_string(),
            meter_aggregation_type: aggregation_type,
            customers: vec![ResourceIdentifier {
                meteroid_id: subscription.customer_id.to_string(),
                external_id: subscription
                    .customer_external_id
                    .clone()
                    .unwrap_or(subscription.customer_id.to_string()), // TODO make mandatory in db, or optional in metering
            }],
            from: Some(date_to_timestamp(usage_period.from)),
            to: Some(date_to_timestamp(usage_period.to)), // exclusive TODO check
            // not used here, defaults to customer_id
            group_by_properties: vec![],
            // the segmentation dimensions TODO
            filter_properties: vec![],
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
        };

        let mut metering_client_mut = self.metering_client.clone();
        let usage_data: QueryMeterResponse =
            metering_client_mut.query_meter(request).await?.into_inner();

        //TODO check that length is 1. Alternatively, do a purpose-built query
        let total = usage_data
            .usage
            .first()
            .and_then(|u| u.value.as_ref())
            .ok_or_else(|| anyhow!("No usage data found"))?;

        let total_usage: Decimal = total.clone().try_into()?;

        Ok(UsageData {
            total_usage,
            usage_details: vec![], // we no longer have the day by day unless we decide to fetch separately. We could do that on demand + on finalize
            period: usage_period,
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
