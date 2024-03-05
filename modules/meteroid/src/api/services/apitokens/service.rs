use meteroid_repository as db;
use tonic::{Request, Response, Status};

use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    parse_uuid,
};

use super::{mapping, ApiTokensServiceComponents};
use meteroid_grpc::meteroid::api::apitokens::v1::{
    api_tokens_service_server::ApiTokensService, CreateApiTokenRequest, CreateApiTokenResponse,
    GetApiTokenByIdRequest, GetApiTokenByIdResponse, ListApiTokensRequest, ListApiTokensResponse,
};
use meteroid_repository::Params;
use nanoid::nanoid;

use crate::api::services::apitokens::error::ApiTokenServiceError;
use crate::api::services::utils::rng::BASE62_ALPHABET;
use crate::eventbus::Event;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use common_grpc::middleware::server::auth::RequestExt;

#[tonic::async_trait]
impl ApiTokensService for ApiTokensServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_api_tokens(
        &self,
        request: Request<ListApiTokensRequest>,
    ) -> Result<Response<ListApiTokensResponse>, Status> {
        let connection = self.get_connection().await?;

        let tenant_id = &request.tenant()?;

        let api_tokens: Vec<db::api_tokens::ApiToken> = db::api_tokens::list_api_tokens()
            .bind(&connection, tenant_id)
            .all()
            .await
            .map_err(|e| {
                ApiTokenServiceError::DatabaseError("unable to list api tokens".to_string(), e)
            })?;

        let result = api_tokens
            .into_iter()
            .map(mapping::api_token::db_to_server)
            .collect();

        Ok(Response::new(ListApiTokensResponse { api_tokens: result }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_api_token(
        &self,
        request: Request<CreateApiTokenRequest>,
    ) -> Result<Response<CreateApiTokenResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

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
                ApiTokenServiceError::PasswordHashError("unable to hash api key".to_string())
            })?
            .to_string();

        // generate a hint that will also be stored
        let hint = format!(
            "{}{}...{}",
            &prefix,
            &api_key_random[..4],
            &id_part[id_part.len() - 4..]
        );

        let params = db::api_tokens::CreateApiTokenParams {
            id,
            name: req.name,
            hint,
            hash: api_key_hash,
            tenant_id,
            created_by: actor,
        };

        let res = db::api_tokens::create_api_token()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                ApiTokenServiceError::DatabaseError("Unable to create api token".to_string(), e)
            })?;

        let _ = self
            .eventbus
            .publish(Event::api_token_created(actor, res.id))
            .await;

        let response = CreateApiTokenResponse {
            api_key,
            details: Some(mapping::api_token::db_to_server(res)),
        };
        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_api_token_by_id(
        &self,
        request: Request<GetApiTokenByIdRequest>,
    ) -> Result<Response<GetApiTokenByIdResponse>, Status> {
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let result: db::api_tokens::GetApiTokenById = db::api_tokens::get_api_token_by_id()
            .bind(&connection, &parse_uuid!(&req.id)?)
            .one()
            .await
            .map_err(|e| {
                ApiTokenServiceError::DatabaseError(
                    "Unable to get api token by hash".to_string(),
                    e,
                )
            })?;

        Ok(Response::new(GetApiTokenByIdResponse {
            tenant_id: result.tenant_id.to_string(),
            hash: result.hash,
        }))
    }
}
