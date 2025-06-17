use crate::api::billablemetrics::mapping::metric::ServerBillableMetricWrapper;
use crate::api::internal::InternalServiceComponents;
use crate::api::internal::error::InternalApiError;
use crate::{api::utils::parse_uuid, parse_uuid};
use common_domain::ids::TenantId;
use error_stack::Report;
use meteroid_grpc::meteroid::api::billablemetrics::v1::BillableMetric;
use meteroid_grpc::meteroid::internal::v1::internal_service_server::InternalService;
use meteroid_grpc::meteroid::internal::v1::{
    ListBillableMetricsRequest, ListBillableMetricsResponse, ResolveApiKeyRequest,
    ResolveApiKeyResponse, ResolveCustomerAliasesRequest, ResolveCustomerAliasesResponse,
    ResolvedId,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use std::collections::HashSet;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl InternalService for InternalServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn resolve_customer_aliases(
        &self,
        request: Request<ResolveCustomerAliasesRequest>,
    ) -> Result<Response<ResolveCustomerAliasesResponse>, Status> {
        let inner = request.into_inner();

        let tenant_id = TenantId::from_proto(inner.tenant_id)?;

        let res = self
            .store
            .find_customer_ids_by_aliases(tenant_id, inner.aliases.clone())
            .await
            .map_err(Into::<InternalApiError>::into)?;

        let mut customers: Vec<ResolvedId> = vec![];

        for record in &res {
            customers.push(ResolvedId {
                alias: record.alias.clone().unwrap(),
                local_id: record.id.to_string(),
            });
        }

        let resolved_aliases: HashSet<String> = res.into_iter().map(|x| x.alias.unwrap()).collect();

        let unresolved_aliases: Vec<String> = inner
            .aliases
            .into_iter()
            .filter(|id| !resolved_aliases.contains(id))
            .collect();

        Ok(Response::new(ResolveCustomerAliasesResponse {
            customers,
            unresolved_aliases,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn resolve_api_key(
        &self,
        request: Request<ResolveApiKeyRequest>,
    ) -> Result<Response<ResolveApiKeyResponse>, Status> {
        let inner = request.into_inner();

        let res = self
            .store
            .get_api_token_by_id_for_validation(&parse_uuid!(inner.api_key_id)?)
            .await
            .map_err(Into::<InternalApiError>::into)?;

        Ok(Response::new(ResolveApiKeyResponse {
            tenant_id: res.tenant_id.to_string(),
            organization_id: res.organization_id.to_string(),
            hash: res.hash,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_billable_metrics(
        &self,
        request: Request<ListBillableMetricsRequest>,
    ) -> Result<Response<ListBillableMetricsResponse>, Status> {
        let inner = request.into_inner();
        let tenant_id = TenantId::from_proto(inner.tenant_id)?;

        let items = self
            .store
            .list_billable_metrics_by_code(tenant_id, inner.code)
            .await
            .map_err(Into::<InternalApiError>::into)?
            .into_iter()
            .map(|bm| ServerBillableMetricWrapper::try_from(bm).map(|v| v.0))
            .collect::<Vec<Result<BillableMetric, Report<StoreError>>>>()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::<InternalApiError>::into)?;

        Ok(Response::new(ListBillableMetricsResponse { items }))
    }
}
