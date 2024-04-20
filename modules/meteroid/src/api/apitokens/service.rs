use tonic::{Request, Response, Status};

use crate::{
    api::utils::{parse_uuid, uuid_gen},
    parse_uuid,
};

use super::{mapping, ApiTokensServiceComponents};
use meteroid_grpc::meteroid::api::apitokens::v1::{
    api_tokens_service_server::ApiTokensService, CreateApiTokenRequest, CreateApiTokenResponse,
    GetApiTokenByIdRequest, GetApiTokenByIdResponse, ListApiTokensRequest, ListApiTokensResponse,
};
use nanoid::nanoid;

use crate::api::apitokens::error::ApiTokenApiError;
use crate::api::utils::rng::BASE62_ALPHABET;
use crate::eventbus::Event;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_store::domain;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;

#[tonic::async_trait]
impl ApiTokensService for ApiTokensServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_api_tokens(
        &self,
        request: Request<ListApiTokensRequest>,
    ) -> Result<Response<ListApiTokensResponse>, Status> {
        let tenant_id = &request.tenant()?;

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

        // TODO
        // api key is ex: ${pv for private key ?? pb for publishable key}_${tenant.env}_ + random
        let prefix = "pv_sand_";

        let id = uuid_gen::v7();

        // encode in base62. Identifier is added to the api key, and used to retrieve the hash.
        let id_part = base62::encode(id.as_u128());

        // Generate the api key
        let api_key_random = nanoid!(28, &BASE62_ALPHABET);
        let api_key = format!("{}{}/{}", &prefix, &api_key_random, &id_part);

        // Generate the hash that we will store in db
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(5 * 1024, 1, 1, None).unwrap(),
        );
        let salt = SaltString::generate(&mut OsRng);
        let api_key_hash = argon2
            .hash_password(&api_key_random.as_bytes(), &salt)
            .map_err(|e| {
                log::error!("Unable to hash api key: {}", e);
                ApiTokenApiError::PasswordHashError("unable to hash api key".to_string())
            })?
            .to_string();

        // generate a hint that will also be stored
        let hint = format!(
            "{}{}...{}",
            &prefix,
            &api_key_random[..4],
            &id_part[id_part.len() - 4..]
        );

        let res = self
            .store
            .insert_api_token(domain::ApiTokenNew {
                id: id,
                name: req.name,
                created_at: chrono::Utc::now().naive_utc(),
                created_by: actor,
                tenant_id,
                hash: api_key_hash,
                hint,
            })
            .await
            .map_err(|e| {
                ApiTokenApiError::StoreError(
                    "Unable to create api token".to_string(),
                    Box::new(e.into_error()),
                )
            })?;

        let _ = self
            .eventbus
            .publish(Event::api_token_created(actor, res.id))
            .await;

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
