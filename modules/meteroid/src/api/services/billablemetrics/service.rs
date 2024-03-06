use cornucopia_async::Params;
use log::error;
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use metering_grpc::meteroid::metering::v1::RegisterMeterRequest;
use meteroid_grpc::meteroid::api::billablemetrics::v1::{
    billable_metrics_service_server::BillableMetricsService, CreateBillableMetricRequest,
    CreateBillableMetricResponse, GetBillableMetricRequest, GetBillableMetricResponse,
    ListBillableMetricsRequest, ListBillableMetricsResponse,
};
use meteroid_repository as db;

use crate::api::services::billablemetrics::error::BillableMetricServiceError;
use crate::api::services::utils::uuid_gen;
use crate::api::services::utils::{parse_uuid, PaginationExt};
use crate::eventbus::Event;

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
        let metric = request.into_inner();
        let connection = self.get_connection().await?;

        let (aggregation_key, aggregation_type, unit_conversion) = match metric.aggregation {
            Some(aggregation) => (
                aggregation.aggregation_key,
                Some(mapping::aggregation_type::server_to_db(
                    aggregation.aggregation_type.try_into().map_err(|e| {
                        BillableMetricServiceError::MappingError(
                            "unknown aggregation_type".to_string(),
                            e,
                        )
                    })?,
                )),
                aggregation.unit_conversion,
            ),
            None => (None, None, None),
        };

        let params = db::billable_metrics::CreateBillableMetricParams {
            id: uuid_gen::v7(),
            product_family_external_id: metric.family_external_id,
            code: metric.code,
            name: metric.name,
            description: metric.description,
            aggregation_key,
            aggregation_type: aggregation_type.unwrap(),
            segmentation_matrix: metric
                .segmentation_matrix
                .map(|s| serde_json::to_value(s).unwrap()),
            // segmentation_matrix_type: None, // TODO remove it from db, we encode it all in segmentation_matrix
            tenant_id, // TODO
            usage_group_key: metric.usage_group_key,
            unit_conversion_factor: Some(1), // TODO float => metric.aggregation.and_then(|a| a.unit_conversion).map(|u| u.factor),
            unit_conversion_rounding: unit_conversion.map(|u| match u.rounding.try_into() {
                Ok(a) => mapping::unit_conversion_rounding::server_to_db(a),
                Err(_) => db::UnitConversionRoundingEnum::NONE,
            }),
            created_by: actor,
        };

        let metric = db::billable_metrics::create_billable_metric()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                error!("Unable to create billable metric: {:#?}", e);
                BillableMetricServiceError::DatabaseError(
                    "unable to create billable metric".to_string(),
                    e,
                )
            })?;

        let rs = mapping::metric::db_to_server(metric.clone());

        let metering_meter = mapping::metric::db_to_metering(metric.clone());

        let _ = &self
            .meters_service_client
            .clone()
            .register_meter(Request::new(RegisterMeterRequest {
                meter: Some(metering_meter),
                tenant_id: tenant_id.to_string(),
            }))
            .await; // TODO add in db/response the register , error and allow retrying

        let _ = self
            .eventbus
            .publish(Event::billable_metric_created(actor, metric.id, tenant_id))
            .await;

        Ok(Response::new(CreateBillableMetricResponse {
            billable_metric: Some(rs),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_billable_metrics(
        &self,
        request: Request<ListBillableMetricsRequest>,
    ) -> Result<Response<ListBillableMetricsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let connection = self.get_connection().await?;

        let params = db::billable_metrics::ListBillableMetricsParams {
            product_family_external_id: inner.family_external_id,
            tenant_id,
            limit: inner.pagination.limit(),
            offset: inner.pagination.offset(),
        };

        let metrics = db::billable_metrics::list_billable_metrics()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                BillableMetricServiceError::DatabaseError(
                    "unable to list billable metrics".to_string(),
                    e,
                )
            })?;

        let total_count = metrics.first().map(|p| p.total_count).unwrap_or(0);
        let rs = metrics
            .into_iter()
            .map(mapping::metric::list_db_to_server)
            .collect::<Vec<_>>();

        Ok(Response::new(ListBillableMetricsResponse {
            billable_metrics: rs,
            pagination_meta: inner.pagination.into_response(total_count as u32),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_billable_metric(
        &self,
        request: Request<GetBillableMetricRequest>,
    ) -> Result<Response<GetBillableMetricResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let connection = self.get_connection().await?;

        let params = db::billable_metrics::GetBillableMetricByIdParams {
            id: parse_uuid(&inner.id, "metric_id")?,
            tenant_id,
        };

        let metric = db::billable_metrics::get_billable_metric_by_id()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                BillableMetricServiceError::DatabaseError(
                    "Unable to get billable metric".to_string(),
                    e,
                )
            })?;

        let rs = mapping::metric::db_to_server(metric);

        Ok(Response::new(GetBillableMetricResponse {
            billable_metric: Some(rs),
        }))
    }
}
