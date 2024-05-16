use std::collections::HashSet;

use tonic::{Request, Response, Status};

use meteroid_grpc::meteroid::internal::v1::internal_service_server::InternalService;
use meteroid_grpc::meteroid::internal::v1::{
    ResolveApiKeyRequest, ResolveApiKeyResponse, ResolveCustomerExternalIdsRequest,
    ResolveCustomerExternalIdsResponse, ResolvedId,
};
use meteroid_store::repositories::api_tokens::ApiTokensInterface;

use crate::api::internal::error::InternalApiError;
use crate::api::internal::InternalServiceComponents;
use crate::{api::utils::parse_uuid, parse_uuid};
use meteroid_store::repositories::customers::CustomersInterface;

#[tonic::async_trait]
impl InternalService for InternalServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn resolve_customer_external_ids(
        &self,
        request: Request<ResolveCustomerExternalIdsRequest>,
    ) -> Result<Response<ResolveCustomerExternalIdsResponse>, Status> {
        let inner = request.into_inner();

        let tenant_id = parse_uuid!(inner.tenant_id)?;

        let res = self
            .store
            .find_customer_ids_by_aliases(tenant_id, inner.external_ids.clone())
            .await
            .map_err(Into::<InternalApiError>::into)?;

        let mut customers: Vec<ResolvedId> = vec![];

        for record in &res {
            customers.push(ResolvedId {
                external_id: record.alias.clone().unwrap(),
                meteroid_id: record.id.to_string(),
            });
        }

        let resolved_aliases: HashSet<String> = res.into_iter().map(|x| x.alias.unwrap()).collect();

        let unresolved_ids: Vec<String> = inner
            .external_ids
            .into_iter()
            .filter(|id| !resolved_aliases.contains(id))
            .collect();

        Ok(Response::new(ResolveCustomerExternalIdsResponse {
            customers,
            unresolved_ids,
        }))
    }

    async fn resolve_api_key(
        &self,
        request: Request<ResolveApiKeyRequest>,
    ) -> Result<Response<ResolveApiKeyResponse>, Status> {
        let inner = request.into_inner();

        let res = self
            .store
            .get_api_token_by_id(&parse_uuid!(inner.api_key_id)?)
            .await
            .map_err(Into::<InternalApiError>::into)?;

        Ok(Response::new(ResolveApiKeyResponse {
            tenant_id: res.tenant_id.to_string(),
            hash: res.hash,
        }))
    }
}
