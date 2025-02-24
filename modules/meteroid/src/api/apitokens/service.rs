use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::apitokens::v1::{
    api_tokens_service_server::ApiTokensService, CreateApiTokenRequest, CreateApiTokenResponse,
    GetApiTokenByIdRequest, GetApiTokenByIdResponse, ListApiTokensRequest, ListApiTokensResponse,
};
use meteroid_store::domain;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;

use crate::api::apitokens::error::ApiTokenApiError;
use crate::{api::utils::parse_uuid, parse_uuid};

use super::{mapping, ApiTokensServiceComponents};

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
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let (api_key, res) = self
            .store
            .insert_api_token(domain::ApiTokenNew {
                name: req.name,
                created_by: actor,
                tenant_id,
            })
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
    async fn get_api_token_by_id(
        &self,
        request: Request<GetApiTokenByIdRequest>,
    ) -> Result<Response<GetApiTokenByIdResponse>, Status> {
        let req = request.into_inner();

        let result = self
            .store
            .get_api_token_by_id(&parse_uuid!(&req.id)?)
            .await
            .map_err(|e| {
                ApiTokenApiError::StoreError(
                    "Unable to get api token by hash".to_string(),
                    Box::new(e.into_error()),
                )
            })?;

        Ok(Response::new(GetApiTokenByIdResponse {
            tenant_id: result.tenant_id.to_string(),
            hash: result.hash,
        }))
    }
}
