use chrono::{NaiveDate, Timelike};
use rust_decimal::Decimal;
use tonic::Request;
use uuid::Uuid;

use crate::api::billablemetrics::mapping;
use common_grpc::middleware::client::LayeredClientService;
use metering_grpc::meteroid::metering::v1::meter::AggregationType;
use metering_grpc::meteroid::metering::v1::meters_service_client::MetersServiceClient;
use metering_grpc::meteroid::metering::v1::query_meter_request::QueryWindowSize;
use metering_grpc::meteroid::metering::v1::usage_query_service_client::UsageQueryServiceClient;
use metering_grpc::meteroid::metering::v1::{
    CustomerIdentifier, Filter, QueryMeterRequest, QueryMeterResponse, RegisterMeterRequest,
};
use meteroid_store::compute::clients::usage::*;
use meteroid_store::compute::ComputeError;
use meteroid_store::domain;
use meteroid_store::domain::{BillableMetric, Period};

#[derive(Clone, Debug)]
pub struct MeteringUsageClient {
    usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
    meters_grpc_client: MetersServiceClient<LayeredClientService>,
}

impl MeteringUsageClient {
    pub fn new(
        usage_grpc_client: UsageQueryServiceClient<LayeredClientService>,
        meters_grpc_client: MetersServiceClient<LayeredClientService>,
    ) -> Self {
        Self {
            usage_grpc_client,
            meters_grpc_client,
        }
    }
}

#[async_trait::async_trait]
impl UsageClient for MeteringUsageClient {
    async fn register_meter(
        &self,
        tenant_id: &Uuid,
        metric: &BillableMetric,
    ) -> Result<Vec<Metadata>, ComputeError> {
        let metering_meter = mapping::metric::domain_to_metering(metric.clone());

        let response = self
            .meters_grpc_client
            .clone()
            .register_meter(Request::new(RegisterMeterRequest {
                meter: Some(metering_meter),
                tenant_id: tenant_id.to_string(),
            }))
            // TODO add in db/response the register , error and allow retrying
            .await
            .map(|r| r.into_inner())
            .map_err(|status| {
                log::error!("Failed to register meter: {:?}", status);
                ComputeError::MeteringGrpcError
            })?;

        let metadata = response
            .metadata
            .into_iter()
            .map(|m| Metadata {
                key: m.key,
                value: m.value,
            })
            .collect::<Vec<Metadata>>();

        Ok(metadata)
    }

    async fn fetch_usage(
        &self,
        tenant_id: &Uuid,
        customer_local_id: &str,
        customer_alias: &Option<String>,
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

        let filter_properties = match metric.segmentation_matrix.clone() {
            Some(domain::SegmentationMatrix::Single(domain::Dimension { key, values })) => {
                vec![Filter {
                    property_name: key,
                    property_value: values,
                }]
            }
            Some(domain::SegmentationMatrix::Double {
                dimension1,
                dimension2,
            }) => {
                vec![
                    Filter {
                        property_name: dimension1.key,
                        property_value: dimension1.values,
                    },
                    Filter {
                        property_name: dimension2.key,
                        property_value: dimension2.values,
                    },
                ]
            }
            Some(domain::SegmentationMatrix::Linked {
                dimension1_key,
                dimension2_key,
                values,
            }) => {
                let mut filter_properties = vec![];
                for (key, values) in values.iter() {
                    filter_properties.push(Filter {
                        property_name: dimension1_key.clone(),
                        property_value: vec![key.clone()],
                    });
                    filter_properties.push(Filter {
                        property_name: dimension2_key.clone(),
                        property_value: values.clone(),
                    });
                }
                filter_properties
            }
            None => vec![],
        };

        let request = QueryMeterRequest {
            tenant_id: tenant_id.to_string(),
            meter_slug: metric.id.to_string(),
            event_name: metric.code.clone(),
            meter_aggregation_type: aggregation_type,
            customers: vec![CustomerIdentifier {
                local_id: customer_local_id.to_string(),
                alias: customer_alias.clone(),
            }],
            from: Some(date_to_timestamp(period.start)),
            to: Some(date_to_timestamp(period.end)), // exclusive TODO check
            // not used here, defaults to customer_id
            group_by_properties: vec![],
            // the segmentation dimensions TODO
            filter_properties,
            window_size: QueryWindowSize::AggregateAll.into(),
            timezone: None,
        };

        let mut metering_client_mut = self.usage_grpc_client.clone();
        let response: QueryMeterResponse = metering_client_mut
            .query_meter(request)
            .await
            .map_err(|status| {
                log::error!("Failed to query meter: {:?}", status);
                ComputeError::MeteringGrpcError
            })?
            .into_inner();

        let data: Vec<GroupedUsageData> = response
            .usage
            .into_iter()
            .map(|usage| {
                let value: Decimal = usage
                    .value
                    .as_ref()
                    .and_then(|v| v.clone().try_into().ok())
                    .unwrap_or(Decimal::ZERO);
                GroupedUsageData {
                    value,
                    dimensions: usage
                        .dimensions
                        .into_iter()
                        .map(|(k, v)| (k, v.value.unwrap_or(String::new())))
                        .collect(),
                }
            })
            .collect();

        Ok(UsageData { data, period })
    }
}

fn date_to_timestamp(dt: NaiveDate) -> prost_types::Timestamp {
    let dt_at_start_of_day = dt.and_hms_opt(0, 0, 0).unwrap();
    prost_types::Timestamp {
        seconds: dt_at_start_of_day.and_utc().timestamp(),
        nanos: dt_at_start_of_day.nanosecond() as i32,
    }
}
