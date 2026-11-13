use crate::billablemetrics::error::BillableMetricApiError;
use crate::billablemetrics::mapping::metric::{
    ServerBillableMetricMetaWrapper, ServerBillableMetricWrapper,
};
use common_domain::identifiers::validate_code;
use common_domain::ids::{BillableMetricId, ProductFamilyId, ProductId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::billablemetrics::v1::{
    ArchiveBillableMetricRequest, ArchiveBillableMetricResponse, BillableMetricMeta,
    CreateBillableMetricRequest, CreateBillableMetricResponse, GetBillableMetricRequest,
    GetBillableMetricResponse, GetBillableMetricUsageRequest, GetBillableMetricUsageResponse,
    ListBillableMetricsRequest, ListBillableMetricsResponse, MetricFilter,
    UnarchiveBillableMetricRequest, UnarchiveBillableMetricResponse, UpdateBillableMetricRequest,
    UpdateBillableMetricResponse, UsageDataPoint, UsageGrain,
    billable_metrics_service_server::BillableMetricsService,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::domain;
use meteroid_store::domain::BillableMetric;
use meteroid_store::domain::misc::UsagePeriod;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::services::clients::usage::TenantUsageGrain;
use rust_decimal::prelude::ToPrimitive;
use tonic::{Request, Response, Status};

use super::{BillableMetricsComponents, mapping};

/// A filter must name a property and carry at least one value — an empty value list is a
/// config error that would otherwise compile to a no-op filter (silently unfiltered rating).
fn validate_metric_filters(filters: &[MetricFilter]) -> Result<(), Status> {
    for f in filters {
        if f.property.trim().is_empty() {
            return Err(Status::invalid_argument(
                "metric filter property must not be empty",
            ));
        }
        if f.values.is_empty() {
            return Err(Status::invalid_argument(format!(
                "metric filter on '{}' must have at least one value",
                f.property
            )));
        }
    }
    Ok(())
}

#[tonic::async_trait]
impl BillableMetricsService for BillableMetricsComponents {
    #[tracing::instrument(skip_all)]
    async fn create_billable_metric(
        &self,
        request: Request<CreateBillableMetricRequest>,
    ) -> Result<Response<CreateBillableMetricResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor_typed = request.actor_typed()?;
        let inner = request.into_inner();

        validate_code(&inner.code).map_err(|e| Status::invalid_argument(e.to_string()))?;
        validate_metric_filters(&inner.filters)?;
        let unit_label = mapping::metric::unit_label_from_server(inner.unit_label.as_ref())?;
        let segmentation_matrix =
            mapping::metric::map_segmentation_matrix_from_server(inner.segmentation_matrix)?;

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
            .insert_billable_metric(
                actor_typed,
                domain::BillableMetricNew {
                    name: inner.name,
                    description: inner.description,
                    code: inner.code,
                    aggregation_type: aggregation_type.unwrap(),
                    aggregation_key,
                    unit_conversion_factor: unit_conversion.as_ref().map(|u| u.factor as i32), // TODO allow float
                    unit_conversion_rounding: unit_conversion.map(|u| {
                        match u.rounding.try_into() {
                            Ok(a) => mapping::unit_conversion_rounding::server_to_domain(a),
                            Err(_) => domain::enums::UnitConversionRoundingEnum::None,
                        }
                    }),
                    segmentation_matrix,
                    filters: mapping::metric::map_filters_from_server(inner.filters),
                    usage_group_key: inner.usage_group_key,
                    unit_label,
                    tenant_id,
                    product_family_id: ProductFamilyId::from_proto(inner.family_local_id)?,
                    product_id: ProductId::from_proto_opt(inner.product_id)?,
                },
            )
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
                inner.order_by,
                inner.search,
            )
            .await
            .map_err(Into::<crate::customers::error::CustomerApiError>::into)?;

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
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let billable_metric_id = BillableMetricId::from_proto(&req.id)?;

        self.store
            .archive_billable_metric(actor, billable_metric_id, tenant_id)
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
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let billable_metric_id = BillableMetricId::from_proto(&inner.id)?;

        let unit_conversion = inner.unit_conversion;

        let segmentation_matrix = if let Some(values_update) = inner.segmentation_matrix_values {
            let existing_metric = self
                .store
                .find_billable_metric_by_id(billable_metric_id, tenant_id)
                .await
                .map_err(Into::<BillableMetricApiError>::into)?;

            mapping::metric::merge_segmentation_matrix_values(
                existing_metric.segmentation_matrix,
                values_update,
            )
            .map(Some)
        } else {
            None
        };

        // Absent = leave filters untouched; present = replace with the given list (empty clears).
        let filters = match inner.filters {
            Some(list) => {
                validate_metric_filters(&list.filters)?;
                Some(mapping::metric::map_filters_from_server(list.filters))
            }
            None => None,
        };

        let unit_label = inner
            .unit_label
            .as_ref()
            .map(|label| mapping::metric::unit_label_from_server(Some(label)))
            .transpose()?;

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
            segmentation_matrix,
            filters,
            unit_label,
        };

        let domain_billable_metric = self
            .store
            .update_billable_metric(actor, billable_metric_id, tenant_id, update)
            .await
            .map_err(Into::<BillableMetricApiError>::into)?;

        let server_billable_metric = ServerBillableMetricWrapper::try_from(domain_billable_metric)
            .map(|v| v.0)
            .map_err(Into::<BillableMetricApiError>::into)?;

        Ok(Response::new(UpdateBillableMetricResponse {
            billable_metric: Some(server_billable_metric),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_billable_metric_usage(
        &self,
        request: Request<GetBillableMetricUsageRequest>,
    ) -> Result<Response<GetBillableMetricUsageResponse>, Status> {
        // Tenant scope comes from the authenticated request, never the body.
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let billable_metric_id = BillableMetricId::from_proto(&req.id)?;

        // Loading scoped by tenant ensures the metric belongs to the caller; its
        // `code` is then the tenant's own event code for the raw-events query.
        let metric = self
            .store
            .find_billable_metric_by_id(billable_metric_id, tenant_id)
            .await
            .map_err(Into::<BillableMetricApiError>::into)?;

        let grain = match req.grain() {
            UsageGrain::Hour => TenantUsageGrain::Hour,
            // Default to day for UNSPECIFIED/DAY.
            _ => TenantUsageGrain::Day,
        };

        let now = common_domain::clock::now().naive_utc();
        // Default span: 30 grains back from now.
        let default_span = match grain {
            TenantUsageGrain::Hour => chrono::Duration::hours(30),
            TenantUsageGrain::Day => chrono::Duration::days(30),
        };

        let parse_date = |s: &str| -> Result<chrono::NaiveDateTime, Status> {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
                .map_err(|e| Status::invalid_argument(format!("invalid date '{s}': {e}")))
        };

        let end = match req.to.as_deref() {
            Some(s) => parse_date(s)?,
            None => now,
        };
        let start = match req.from.as_deref() {
            Some(s) => parse_date(s)?,
            None => end - default_span,
        };

        if start >= end {
            return Err(Status::invalid_argument(
                "`from` must be strictly before `to`",
            ));
        }

        let period = UsagePeriod { start, end };

        let points = self
            .usage_client
            .fetch_tenant_usage(&tenant_id, &metric, period, grain)
            .await
            .map_err(|e| Status::internal(format!("Failed to fetch metric usage: {e}")))?;

        let data = points
            .into_iter()
            .map(|p| UsageDataPoint {
                window_start: p.window_start.and_utc().to_rfc3339(),
                window_end: p.window_end.and_utc().to_rfc3339(),
                value: p.value.to_f64().unwrap_or(0.0),
            })
            .collect();

        Ok(Response::new(GetBillableMetricUsageResponse { data }))
    }
}

#[cfg(test)]
mod validate_metric_filters_tests {
    use super::*;

    fn filter(property: &str, values: &[&str]) -> MetricFilter {
        MetricFilter {
            property: property.to_string(),
            op: 0,
            values: values.iter().map(|v| v.to_string()).collect(),
        }
    }

    #[test]
    fn rejects_blank_property() {
        assert!(validate_metric_filters(&[filter("   ", &["gpt_4"])]).is_err());
    }

    #[test]
    fn rejects_empty_values() {
        assert!(validate_metric_filters(&[filter("model", &[])]).is_err());
    }

    #[test]
    fn accepts_property_with_values() {
        assert!(validate_metric_filters(&[filter("model", &["gpt_4"])]).is_ok());
    }
}
