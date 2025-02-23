use std::collections::HashSet;

use common_domain::ids::TenantId;
use meteroid_grpc::meteroid::internal::v1::internal_service_server::InternalService;
use meteroid_grpc::meteroid::internal::v1::{
    ResolveApiKeyRequest, ResolveApiKeyResponse, ResolveCustomerAliasesRequest,
    ResolveCustomerAliasesResponse, ResolvedId,
};
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use tonic::{Request, Response, Status};

use crate::api::internal::error::InternalApiError;
use crate::api::internal::InternalServiceComponents;
use crate::{api::utils::parse_uuid, parse_uuid};
use meteroid_store::repositories::customers::CustomersInterface;

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
}
