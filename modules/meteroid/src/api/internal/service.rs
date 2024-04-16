use std::collections::HashSet;

use tonic::{Request, Response, Status};

use meteroid_grpc::meteroid::internal::v1::internal_service_server::InternalService;
use meteroid_grpc::meteroid::internal::v1::{
    ResolveApiKeyRequest, ResolveApiKeyResponse, ResolveCustomerExternalIdsRequest,
    ResolveCustomerExternalIdsResponse, ResolvedId,
};
use meteroid_repository as db;

use crate::api::internal::error::InternalApiError;
use crate::{api::utils::parse_uuid, db::DbService, parse_uuid};

#[tonic::async_trait]
impl InternalService for DbService {
    #[tracing::instrument(skip_all)]
    async fn resolve_customer_external_ids(
        &self,
        request: Request<ResolveCustomerExternalIdsRequest>,
    ) -> Result<Response<ResolveCustomerExternalIdsResponse>, Status> {
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let res = db::customers::get_customer_ids_by_alias()
            .bind(&connection, &parse_uuid!(req.tenant_id)?, &req.external_ids)
            .all()
            .await
            .map_err(|e| {
                InternalApiError::DatabaseError(
                    "unable to resolve customer external ids".to_string(),
                    e,
                )
            })?;

        let mut customers: Vec<ResolvedId> = vec![];

        for record in &res {
            customers.push(ResolvedId {
                external_id: record.alias.clone(),
                meteroid_id: record.id.to_string(),
            });
        }

        let resolved_aliases: HashSet<String> = res.into_iter().map(|x| x.alias).collect();
        let unresolved_ids: Vec<String> = req
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
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let res = db::api_tokens::get_api_token_by_id()
            .bind(&connection, &parse_uuid!(req.api_key_id)?)
            .one()
            .await
            .map_err(|e| {
                InternalApiError::DatabaseError("unable to resolve api key".to_string(), e)
            })?;

        Ok(Response::new(ResolveApiKeyResponse {
            tenant_id: res.tenant_id.to_string(),
            hash: res.hash,
        }))
    }
}
