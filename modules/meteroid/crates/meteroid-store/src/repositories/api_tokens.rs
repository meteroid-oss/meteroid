use crate::domain::ApiTokenValidation;
use crate::domain::api_tokens::ApiToken;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::TenantEnvironmentEnum;
use crate::errors::StoreError;
use crate::store::Store;
use crate::{StoreResult, domain};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use common_domain::ids::{ApiTokenId, BaseId, TenantId};
use common_eventbus::Event;
use diesel_models::api_tokens::{ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow};
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use nanoid::nanoid;
use scoped_futures::ScopedFutureExt;
use tracing_log::log;

#[async_trait::async_trait]
pub trait ApiTokensInterface {
    async fn find_api_tokens_by_tenant_id(&self, tenant_id: TenantId)
    -> StoreResult<Vec<ApiToken>>;

    async fn get_api_token_by_id(&self, id: &ApiTokenId) -> StoreResult<ApiToken>;

    async fn get_api_token_by_id_for_validation(
        &self,
        id: &ApiTokenId,
    ) -> StoreResult<ApiTokenValidation>;

    async fn insert_api_token(
        &self,
        actor: Actor,
        plan: domain::ApiTokenNew,
    ) -> StoreResult<(String, ApiToken)>;

    async fn delete_api_token(
        &self,
        actor: Actor,
        id: &ApiTokenId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl ApiTokensInterface for Store {
    async fn find_api_tokens_by_tenant_id(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ApiToken>> {
        let mut conn = self.get_conn().await?;

        let api_tokens = ApiTokenRow::find_by_tenant_id(&mut conn, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_tokens.into_iter().map(Into::into).collect())
    }

    async fn get_api_token_by_id(&self, id: &ApiTokenId) -> StoreResult<ApiToken> {
        let mut conn = self.get_conn().await?;

        let api_token = ApiTokenRow::find_by_id(&mut conn, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_token.into())
    }

    async fn get_api_token_by_id_for_validation(
        &self,
        id: &ApiTokenId,
    ) -> StoreResult<ApiTokenValidation> {
        let mut conn = self.get_conn().await?;

        let api_token = ApiTokenValidationRow::find_by_id(&mut conn, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_token.into())
    }

    async fn insert_api_token(
        &self,
        actor: Actor,
        entity: domain::ApiTokenNew,
    ) -> StoreResult<(String, ApiToken)> {
        let mut conn = self.get_conn().await?;

        let id = ApiTokenId::new();

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
            .hash_password(api_key_random.as_bytes(), &salt)
            .map_err(|e| {
                log::error!("Unable to hash api key: {e}");
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
            tenant_id: entity.tenant_id,
            hash: api_key_hash,
            hint,
        };

        let res: ApiToken = self
            .transaction(|conn| {
                let actor = &actor;
                let entity = &insertable_entity;
                async move {
                    let res: ApiToken = entity
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .map(Into::into)?;

                    let activity = Activity::new(
                        ActivityType::ApiTokenCreated,
                        EntityType::ApiToken,
                        entity.id.as_uuid(),
                    )
                    .with_metadata(serde_json::json!({
                        "name": entity.name,
                        "hint": entity.hint,
                    }));
                    self.internal
                        .record_audit_tx(
                            conn,
                            entity.tenant_id,
                            actor,
                            AuditInput::Activity(activity),
                        )
                        .await?;
                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::api_token_created(
                actor.clone(),
                insertable_entity.id,
                insertable_entity.tenant_id,
            ))
            .await;

        Ok((api_key, res))
    }

    async fn delete_api_token(
        &self,
        actor: Actor,
        id: &ApiTokenId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                ApiTokenRow::delete_by_id(conn, id, tenant_id)
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;

                let activity = Activity::new(
                    ActivityType::ApiTokenRevoked,
                    EntityType::ApiToken,
                    id.as_uuid(),
                );
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await
            }
            .scope_boxed()
        })
        .await?;

        let _ = self
            .eventbus
            .publish(Event::api_token_revoked(actor.clone(), *id, tenant_id))
            .await;

        Ok(())
    }
}
