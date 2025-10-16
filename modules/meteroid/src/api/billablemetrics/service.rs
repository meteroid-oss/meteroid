use crate::api::billablemetrics::error::BillableMetricApiError;
use crate::api::billablemetrics::mapping::metric::{
    ServerBillableMetricMetaWrapper, ServerBillableMetricWrapper,
};
use crate::api::utils::PaginationExt;
use common_domain::ids::{BillableMetricId, ProductFamilyId, ProductId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::billablemetrics::v1::{
    ArchiveBillableMetricRequest, ArchiveBillableMetricResponse, BillableMetricMeta,
    CreateBillableMetricRequest, CreateBillableMetricResponse, GetBillableMetricRequest,
    GetBillableMetricResponse, ListBillableMetricsRequest, ListBillableMetricsResponse,
    UnarchiveBillableMetricRequest, UnarchiveBillableMetricResponse, UpdateBillableMetricRequest,
    UpdateBillableMetricResponse, billable_metrics_service_server::BillableMetricsService,
};
use meteroid_store::domain;
use meteroid_store::domain::BillableMetric;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use tonic::{Request, Response, Status};

use super::{BillableMetricsComponents, mapping};

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

        let (aggregation_key, aggregation_type, unit_conversion) = match inner.aggregation.as_ref()
        {
            Some(aggregation) => (
                aggregation.aggregation_key.clone(),
                Some(mapping::aggregation_type::server_to_domain(
                    aggregation.aggregation_type(),
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
                unit_conversion_factor: unit_conversion.as_ref().map(|u| u.factor as i32), // TODO allow float
                unit_conversion_rounding: unit_conversion.map(|u| match u.rounding.try_into() {
                    Ok(a) => mapping::unit_conversion_rounding::server_to_domain(a),
                    Err(_) => domain::enums::UnitConversionRoundingEnum::None,
                }),
                segmentation_matrix: mapping::metric::map_segmentation_matrix_from_server(
                    inner.segmentation_matrix,
                ),
                usage_group_key: inner.usage_group_key,
                created_by: actor,
                tenant_id,
                product_family_id: ProductFamilyId::from_proto(inner.family_local_id)?,
                product_id: ProductId::from_proto_opt(inner.product_id)?,
            })
            .await
            .map_err(Into::<BillableMetricApiError>::into)?;

        let server_billable_metric =
            ServerBillableMetricWrapper::try_from(domain_billable_metric.clone())
                .map(|v| v.0)
                .map_err(Into::<BillableMetricApiError>::into)?;

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

        let pagination_req = inner.pagination.into_domain();

        let res = self
            .store
            .list_billable_metrics(
                tenant_id,
                pagination_req,
                ProductFamilyId::from_proto_opt(inner.family_local_id)?,
                inner.archived,
            )
            .await
            .map_err(Into::<crate::api::customers::error::CustomerApiError>::into)?;

        let response = ListBillableMetricsResponse {
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
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

        let billable_metric_id = BillableMetricId::from_proto(&req.id)?;

        let billable_metric = self
            .store
            .find_billable_metric_by_id(billable_metric_id, tenant_id)
            .await
            .and_then(ServerBillableMetricWrapper::try_from)
            .map(|v| v.0)
            .map_err(Into::<BillableMetricApiError>::into)?;

        Ok(Response::new(GetBillableMetricResponse {
            billable_metric: Some(billable_metric),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_billable_metric(
        &self,
        request: Request<ArchiveBillableMetricRequest>,
    ) -> Result<Response<ArchiveBillableMetricResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let billable_metric_id = BillableMetricId::from_proto(&req.id)?;

        self.store
            .archive_billable_metric(billable_metric_id, tenant_id)
            .await
            .map_err(Into::<BillableMetricApiError>::into)?;

        Ok(Response::new(ArchiveBillableMetricResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn unarchive_billable_metric(
        &self,
        request: Request<UnarchiveBillableMetricRequest>,
    ) -> Result<Response<UnarchiveBillableMetricResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let billable_metric_id = BillableMetricId::from_proto(&req.id)?;

        self.store
            .unarchive_billable_metric(billable_metric_id, tenant_id)
            .await
            .map_err(Into::<BillableMetricApiError>::into)?;

        Ok(Response::new(UnarchiveBillableMetricResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn update_billable_metric(
        &self,
        request: Request<UpdateBillableMetricRequest>,
    ) -> Result<Response<UpdateBillableMetricResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let billable_metric_id = BillableMetricId::from_proto(&inner.id)?;

        let unit_conversion = inner.unit_conversion;

        let update = domain::BillableMetricUpdate {
            name: inner.name,
            description: inner.description.map(Some),
            unit_conversion_factor: unit_conversion.as_ref().map(|u| Some(u.factor as i32)),
            unit_conversion_rounding: unit_conversion.map(|u| {
                Some(match u.rounding.try_into() {
                    Ok(a) => mapping::unit_conversion_rounding::server_to_domain(a),
                    Err(_) => domain::enums::UnitConversionRoundingEnum::None,
                })
            }),
            segmentation_matrix: inner
                .segmentation_matrix
                .map(|s| mapping::metric::map_segmentation_matrix_from_server(Some(s))),
        };

        let domain_billable_metric = self
            .store
            .update_billable_metric(billable_metric_id, tenant_id, update)
            .await
            .map_err(Into::<BillableMetricApiError>::into)?;

        let server_billable_metric = ServerBillableMetricWrapper::try_from(domain_billable_metric)
            .map(|v| v.0)
            .map_err(Into::<BillableMetricApiError>::into)?;

        Ok(Response::new(UpdateBillableMetricResponse {
            billable_metric: Some(server_billable_metric),
        }))
    }
}
