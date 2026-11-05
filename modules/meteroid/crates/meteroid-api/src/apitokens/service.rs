use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::apitokens::v1::{
    CreateApiTokenRequest, CreateApiTokenResponse, ListApiTokensRequest, ListApiTokensResponse,
    RevokeApiTokenRequest, RevokeApiTokenResponse, api_tokens_service_server::ApiTokensService,
};
use meteroid_store::domain;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;

use crate::apitokens::error::ApiTokenApiError;

use super::{ApiTokensServiceComponents, mapping};

#[tonic::async_trait]
impl ApiTokensService for ApiTokensServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_api_tokens(
        &self,
        request: Request<ListApiTokensRequest>,
    ) -> Result<Response<ListApiTokensResponse>, Status> {
        let tenant_id = request.tenant()?;

        let domain_api_tokens: Vec<domain::api_tokens::ApiToken> = self
            .store
            .find_api_tokens_by_tenant_id(tenant_id)
            .await
            .map_err(|e| {
                ApiTokenApiError::StoreError(
                    "unable to list api tokens".to_string(),
                    Box::new(e.into_error()),
                )
            })?;

        let result = domain_api_tokens
            .into_iter()
            .map(mapping::api_token::domain_to_api)
            .collect();

        Ok(Response::new(ListApiTokensResponse { api_tokens: result }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_api_token(
        &self,
        request: Request<CreateApiTokenRequest>,
    ) -> Result<Response<CreateApiTokenResponse>, Status> {
        let actor_typed = request.actor_typed()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let (api_key, res) = self
            .store
            .insert_api_token(
                actor_typed,
                domain::ApiTokenNew {
                    name: req.name,
                    tenant_id,
                },
            )
            .await
            .map_err(|e| {
                ApiTokenApiError::StoreError(
                    "Unable to create api token".to_string(),
                    Box::new(e.into_error()),
                )
            })?;

        let response = CreateApiTokenResponse {
            api_key,
            details: Some(mapping::api_token::domain_to_api(res)),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn revoke_api_token(
        &self,
        request: Request<RevokeApiTokenRequest>,
    ) -> Result<Response<RevokeApiTokenResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let token_id = common_domain::ids::ApiTokenId::from_proto(&req.id)?;

        self.store
            .delete_api_token(actor, &token_id, tenant_id)
            .await
            .map_err(|e| {
                ApiTokenApiError::StoreError(
                    "Unable to revoke api token".to_string(),
                    Box::new(e.into_error()),
                )
            })?;

        Ok(Response::new(RevokeApiTokenResponse {}))
    }
}
