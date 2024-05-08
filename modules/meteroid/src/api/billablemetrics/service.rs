use error_stack::Report;
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use metering_grpc::meteroid::metering::v1::RegisterMeterRequest;
use meteroid_grpc::meteroid::api::billablemetrics::v1::{
    billable_metrics_service_server::BillableMetricsService, BillableMetricMeta,
    CreateBillableMetricRequest, CreateBillableMetricResponse, GetBillableMetricRequest,
    GetBillableMetricResponse, ListBillableMetricsRequest, ListBillableMetricsResponse,
};
use meteroid_store::domain;
use meteroid_store::domain::BillableMetric;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;

use crate::api::billablemetrics::error::BillableMetricApiError;
use crate::api::billablemetrics::mapping::metric::{
    ServerBillableMetricMetaWrapper, ServerBillableMetricWrapper,
};
use crate::api::utils::{parse_uuid, PaginationExt};

use super::{mapping, BillableMetricsComponents};

#[tonic::async_trait]
impl BillableMetricsService for BillableMetricsComponents {
    #[tracing::instrument(skip_all)]
    async fn create_billable_metric(
        &self,
        request: Request<CreateBillableMetricRequest>,
    ) -> Result<Response<CreateBillableMetricResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let inner = request.into_inner();

        let (aggregation_key, aggregation_type, unit_conversion) = match inner.aggregation {
            Some(aggregation) => (
                aggregation.aggregation_key,
                Some(mapping::aggregation_type::server_to_domain(
                    aggregation.aggregation_type.try_into().map_err(|e| {
                        BillableMetricApiError::MappingError(
                            "unknown aggregation_type".to_string(),
                            e,
                        )
                    })?,
                )),
                aggregation.unit_conversion,
            ),
            None => (None, None, None),
        };

        let domain_billable_metric: BillableMetric = self
            .store
            .insert_billable_metric(domain::BillableMetricNew {
                name: inner.name,
                description: inner.description,
                code: inner.code,
                aggregation_type: aggregation_type.unwrap(),
                aggregation_key,
                unit_conversion_factor: Some(1), // todo fix?
                unit_conversion_rounding: unit_conversion.map(|u| match u.rounding.try_into() {
                    Ok(a) => mapping::unit_conversion_rounding::server_to_domain(a),
                    Err(_) => domain::enums::UnitConversionRoundingEnum::None,
                }),
                segmentation_matrix: inner
                    .segmentation_matrix
                    .map(|s| serde_json::to_value(s).unwrap()),
                usage_group_key: inner.usage_group_key,
                created_by: actor,
                tenant_id,
                family_external_id: inner.family_external_id,
            })
            .await
            .map_err(Into::<BillableMetricApiError>::into)?;

        let server_billable_metric =
            ServerBillableMetricWrapper::try_from(domain_billable_metric.clone())
                .map(|v| v.0)
                .map_err(Into::<BillableMetricApiError>::into)?;

        let metering_meter = mapping::metric::domain_to_metering(domain_billable_metric.clone());

        let _ = &self
            .meters_service_client
            .clone()
            .register_meter(Request::new(RegisterMeterRequest {
                meter: Some(metering_meter),
                tenant_id: tenant_id.to_string(),
            }))
            .await; // TODO add in db/response the register , error and allow retrying

        Ok(Response::new(CreateBillableMetricResponse {
            billable_metric: Some(server_billable_metric),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_billable_metrics(
        &self,
        request: Request<ListBillableMetricsRequest>,
    ) -> Result<Response<ListBillableMetricsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let pagination_req = domain::PaginationRequest {
            page: inner.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: inner.pagination.as_ref().map(|p| p.limit),
        };

        let res = self
            .store
            .list_billable_metrics(tenant_id, pagination_req, inner.family_external_id)
            .await
            .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?;

        let response = ListBillableMetricsResponse {
            pagination_meta: inner.pagination.into_response(res.total_results as u32),
            billable_metrics: res
                .items
                .into_iter()
                .map(|l| ServerBillableMetricMetaWrapper::try_from(l).map(|v| v.0))
                .collect::<Vec<Result<BillableMetricMeta, Report<StoreError>>>>()
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::<BillableMetricApiError>::into)?,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_billable_metric(
        &self,
        request: Request<GetBillableMetricRequest>,
    ) -> Result<Response<GetBillableMetricResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let billable_metric_id = parse_uuid(&req.id, "id")?;

        let billable_metric = self
            .store
            .find_billable_metric_by_id(billable_metric_id.clone(), tenant_id.clone())
            .await
            .and_then(ServerBillableMetricWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<BillableMetricApiError>::into)?;

        Ok(Response::new(GetBillableMetricResponse {
            billable_metric: Some(billable_metric),
        }))
    }
}
