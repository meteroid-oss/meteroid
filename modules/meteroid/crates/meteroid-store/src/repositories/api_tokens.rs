use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2,
};
use common_eventbus::Event;
use diesel_models::api_tokens::{ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow};
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use nanoid::nanoid;
use tracing_log::log;
use uuid::Uuid;

use crate::domain::api_tokens::ApiToken;
use crate::domain::enums::TenantEnvironmentEnum;
use crate::domain::ApiTokenValidation;
use crate::errors::StoreError;
use crate::store::Store;
use crate::{domain, StoreResult};

#[async_trait::async_trait]
pub trait ApiTokensInterface {
    async fn find_api_tokens_by_tenant_id(
        &self,
        tenant_id: &uuid::Uuid,
    ) -> StoreResult<Vec<ApiToken>>;

    async fn get_api_token_by_id(&self, id: &uuid::Uuid) -> StoreResult<ApiToken>;

    async fn get_api_token_by_id_for_validation(
        &self,
        id: &Uuid,
    ) -> StoreResult<ApiTokenValidation>;

    async fn insert_api_token(&self, plan: domain::ApiTokenNew) -> StoreResult<(String, ApiToken)>;
}

#[async_trait::async_trait]
impl ApiTokensInterface for Store {
    async fn find_api_tokens_by_tenant_id(
        &self,
        tenant_id: &uuid::Uuid,
    ) -> StoreResult<Vec<ApiToken>> {
        let mut conn = self.get_conn().await?;

        let api_tokens = ApiTokenRow::find_by_tenant_id(&mut conn, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_tokens.into_iter().map(Into::into).collect())
    }

    async fn get_api_token_by_id(&self, id: &Uuid) -> StoreResult<ApiToken> {
        let mut conn = self.get_conn().await?;

        let api_token = ApiTokenRow::find_by_id(&mut conn, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_token.into())
    }

    async fn get_api_token_by_id_for_validation(
        &self,
        id: &Uuid,
    ) -> StoreResult<ApiTokenValidation> {
        let mut conn = self.get_conn().await?;

        let api_token = ApiTokenValidationRow::find_by_id(&mut conn, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_token.into())
    }

    async fn insert_api_token(
        &self,
        entity: domain::ApiTokenNew,
    ) -> StoreResult<(String, ApiToken)> {
        let mut conn = self.get_conn().await?;

        let id = Uuid::now_v7();

        let tenant = TenantRow::find_by_id(&mut conn, entity.tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let env: TenantEnvironmentEnum = tenant.environment.into();

        // api key is ex: ${pv for private key ?? pb for publishable key}_${tenant.env}_ + random
        let prefix = format!("pv_{}_", env.as_short_string());

        // encode in base62. Identifier is added to the api key, and used to retrieve the hash.
        let id_part = base62::encode(id.as_u128());

        // Generate the api key
        let api_key_random = nanoid!(28, &common_utils::rng::BASE62_ALPHABET);
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
                StoreError::InvalidArgument("unable to hash api key".to_string())
            })?
            .to_string();

        // generate a hint that will also be stored
        let hint = format!(
            "{}{}...{}",
            &prefix,
            &api_key_random[..4],
            &id_part[id_part.len() - 4..]
        );

        let insertable_entity = ApiTokenRowNew {
            id,
            name: entity.name,
            created_at: chrono::Utc::now().naive_utc(),
            created_by: entity.created_by,
            tenant_id: entity.tenant_id,
            hash: api_key_hash,
            hint,
        };

        let result: Result<ApiToken, Report<StoreError>> = insertable_entity
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into);

        if result.is_ok() {
            let _ = self
                .eventbus
                .publish(Event::api_token_created(
                    insertable_entity.created_by,
                    insertable_entity.id,
                ))
                .await;
        }

        result.map(|res| (api_key, res))
    }
}
