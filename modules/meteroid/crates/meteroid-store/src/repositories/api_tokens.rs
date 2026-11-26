use crate::domain::ApiTokenValidation;
use crate::domain::api_tokens::{ApiToken, PublishableKey};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::{ApiTokenKind, TenantEnvironmentEnum};
use crate::errors::StoreError;
use crate::store::Store;
use crate::{StoreResult, domain};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use common_domain::ids::{ApiTokenId, BaseId, TenantId};
use common_eventbus::Event;
use diesel_models::api_tokens::{
    ApiTokenRow, ApiTokenRowNew, ApiTokenValidationRow, PublishableKeyRow,
};
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use nanoid::nanoid;
use scoped_futures::ScopedFutureExt;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use tracing_log::log;

#[async_trait::async_trait]
pub trait ApiTokensInterface {
    async fn find_api_tokens_by_tenant_id(&self, tenant_id: TenantId)
    -> StoreResult<Vec<ApiToken>>;

    async fn get_api_token_by_id_for_validation(
        &self,
        id: &ApiTokenId,
    ) -> StoreResult<ApiTokenValidation>;

    /// The key of this id, whatever its tenant: the key a request authenticated with, which may
    /// act for another tenant (a connected account).
    async fn find_authenticated_api_token(&self, id: ApiTokenId) -> StoreResult<Option<ApiToken>>;

    async fn insert_api_token(
        &self,
        actor: Actor,
        plan: domain::ApiTokenNew,
    ) -> StoreResult<(String, ApiToken)>;

    /// Installs a caller-supplied key. `id` is the token id the key already encodes, so
    /// auth resolves this very row from it, and `secret` is the key's random segment,
    /// hashed here exactly as a generated key's is.
    async fn insert_api_token_with_key(
        &self,
        actor: Actor,
        entity: domain::ApiTokenNew,
        id: ApiTokenId,
        secret: SecretString,
    ) -> StoreResult<ApiToken>;

    /// A publishable key (`pk_{env}_…`): read-only and anonymous, it authenticates the client
    /// API's public routes only, from the `allowed_origins` when any (normalized as the browser
    /// reports an origin). Not a secret: it is stored whole and listed again.
    async fn insert_publishable_key(
        &self,
        actor: Actor,
        entity: domain::ApiTokenNew,
        allowed_origins: Vec<String>,
    ) -> StoreResult<ApiToken>;

    /// The publishable key presented, none when it is not one (a secret key included).
    async fn authenticate_publishable_key(&self, key: &str) -> StoreResult<Option<PublishableKey>>;

    async fn report_api_token_usage(&self, id: &ApiTokenId) -> StoreResult<()>;

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

    async fn find_authenticated_api_token(&self, id: ApiTokenId) -> StoreResult<Option<ApiToken>> {
        let mut conn = self.get_conn().await?;
        Ok(ApiTokenRow::find_by_id(&mut conn, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .map(Into::into))
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

    async fn insert_publishable_key(
        &self,
        actor: Actor,
        entity: domain::ApiTokenNew,
        allowed_origins: Vec<String>,
    ) -> StoreResult<ApiToken> {
        let allowed_origins = crate::domain::tenants::normalize_embed_origins(allowed_origins)
            .map_err(Report::new)?;
        let env = tenant_environment(self, entity.tenant_id).await?;

        let id = ApiTokenId::new();
        let secret = nanoid!(API_KEY_SECRET_LEN, &common_utils::rng::BASE62_ALPHABET);
        let id_part = base62::encode(id.as_u128());
        let prefix = publishable_key_prefix(&env);
        let key = format!("{prefix}{secret}/{id_part}");

        let row = ApiTokenRowNew {
            id,
            name: entity.name,
            created_at: common_clock::now().naive_utc(),
            tenant_id: entity.tenant_id,
            // No secret to check: no argon2 hash, and an empty one verifies nothing.
            hash: String::new(),
            hint: hint(&prefix, &secret, &id_part),
            kind: ApiTokenKind::Publishable.into(),
            allowed_origins,
            publishable_key: Some(key),
        };
        insert_row(self, actor, row).await
    }

    async fn authenticate_publishable_key(&self, key: &str) -> StoreResult<Option<PublishableKey>> {
        let Some(id) = publishable_key_id(key) else {
            return Ok(None);
        };
        let mut conn = self.get_conn().await?;

        let row = PublishableKeyRow::find_by_id(&mut conn, &id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        // The id is readable in the key: the whole key must match, compared by digest so the
        // time taken tells nothing of the stored one.
        Ok(row
            .filter(|row| {
                row.publishable_key.as_deref().is_some_and(|stored| {
                    Sha256::digest(stored.as_bytes()) == Sha256::digest(key.as_bytes())
                })
            })
            .map(PublishableKey::from))
    }

    async fn report_api_token_usage(&self, id: &ApiTokenId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        ApiTokenRow::record_usage(&mut conn, id, common_clock::now().naive_utc())
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(())
    }

    async fn insert_api_token(
        &self,
        actor: Actor,
        entity: domain::ApiTokenNew,
    ) -> StoreResult<(String, ApiToken)> {
        let env = tenant_environment(self, entity.tenant_id).await?;

        let id = ApiTokenId::new();
        let secret = nanoid!(28, &common_utils::rng::BASE62_ALPHABET);
        let api_key = format!(
            "{}{}/{}",
            key_prefix(&env),
            &secret,
            base62::encode(id.as_u128())
        );

        let token = insert_api_token_row(self, actor, entity, id, &secret, &env).await?;

        Ok((api_key, token))
    }

    async fn insert_api_token_with_key(
        &self,
        actor: Actor,
        entity: domain::ApiTokenNew,
        id: ApiTokenId,
        secret: SecretString,
    ) -> StoreResult<ApiToken> {
        let env = tenant_environment(self, entity.tenant_id).await?;

        insert_api_token_row(self, actor, entity, id, secret.expose_secret(), &env).await
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

/// A secret key is `pv_{tenant.env}_{random}/{base62 id}`.
pub fn key_prefix(env: &TenantEnvironmentEnum) -> String {
    format!("pv_{}_", env.as_short_string())
}

/// What every publishable key starts with: `pk_{tenant.env}_{random}/{base62 id}`.
pub const PUBLISHABLE_KEY_PREFIX: &str = "pk_";

pub fn publishable_key_prefix(env: &TenantEnvironmentEnum) -> String {
    format!("{PUBLISHABLE_KEY_PREFIX}{}_", env.as_short_string())
}

pub fn is_publishable_key(key: &str) -> bool {
    key.starts_with(PUBLISHABLE_KEY_PREFIX)
}

/// The token id a publishable key encodes after its last `/`.
fn publishable_key_id(key: &str) -> Option<ApiTokenId> {
    if !is_publishable_key(key) {
        return None;
    }
    let (_, id_part) = key.rsplit_once('/')?;
    let id = base62::decode(id_part).ok()?;
    Some(ApiTokenId::from_const(uuid::Uuid::from_u128(id)))
}

fn hint(prefix: &str, secret: &str, id_part: &str) -> String {
    format!(
        "{}{}...{}",
        prefix,
        &secret[..4],
        &id_part[id_part.len().saturating_sub(4)..]
    )
}

/// Width of the random segment we mint. A caller-supplied key is held to the same bar: a
/// shorter one would be brute-forceable, and the stored hint exposes its first 4 characters.
pub const API_KEY_SECRET_LEN: usize = 28;

/// Applies to a supplied key as much as a minted one, so the hint can slice bytes safely and
/// no path installs a weak credential.
pub fn validate_api_key_secret(secret: &str) -> StoreResult<()> {
    if secret.len() < API_KEY_SECRET_LEN || !secret.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(StoreError::InvalidArgument(format!(
            "api key secret must be at least {API_KEY_SECRET_LEN} ASCII alphanumeric characters"
        ))
        .into());
    }

    Ok(())
}

async fn tenant_environment(
    store: &Store,
    tenant_id: TenantId,
) -> StoreResult<TenantEnvironmentEnum> {
    let mut conn = store.get_conn().await?;

    let tenant = TenantRow::find_by_id(&mut conn, tenant_id)
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

    Ok(tenant.environment.into())
}

/// The single hashing path: a key we mint and a key an operator supplies are stored the
/// same way, so both validate through `ApiTokenValidator`.
async fn insert_api_token_row(
    store: &Store,
    actor: Actor,
    entity: domain::ApiTokenNew,
    id: ApiTokenId,
    secret: &str,
    env: &TenantEnvironmentEnum,
) -> StoreResult<ApiToken> {
    validate_api_key_secret(secret)?;

    let prefix = key_prefix(env);

    // encode in base62. Identifier is added to the api key, and used to retrieve the hash.
    let id_part = base62::encode(id.as_u128());

    // Generate the hash that we will store in db
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(5 * 1024, 1, 1, None).unwrap(),
    );
    let salt = SaltString::generate(&mut OsRng);
    let api_key_hash = argon2
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| {
            log::error!("Unable to hash api key: {e}");
            StoreError::InvalidArgument("unable to hash api key".to_string())
        })?
        .to_string();

    // generate a hint that will also be stored
    let hint = hint(&prefix, secret, &id_part);

    let insertable_entity = ApiTokenRowNew {
        id,
        name: entity.name,
        created_at: common_clock::now().naive_utc(),
        tenant_id: entity.tenant_id,
        hash: api_key_hash,
        hint,
        kind: ApiTokenKind::Secret.into(),
        allowed_origins: vec![],
        publishable_key: None,
    };

    insert_row(store, actor, insertable_entity).await
}

/// Inserts the key with its audit entry, then announces it.
async fn insert_row(
    store: &Store,
    actor: Actor,
    insertable_entity: ApiTokenRowNew,
) -> StoreResult<ApiToken> {
    let res: ApiToken = store
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
                store
                    .internal
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

    let _ = store
        .eventbus
        .publish(Event::api_token_created(
            actor.clone(),
            insertable_entity.id,
            insertable_entity.tenant_id,
        ))
        .await;

    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_publishable_key_names_a_publishable_key_id() {
        let id = ApiTokenId::new();
        let id_part = base62::encode(id.as_u128());
        let secret = "s".repeat(28);
        assert_eq!(
            publishable_key_id(&format!("pk_sand_{secret}/{id_part}")),
            Some(id)
        );
        assert_eq!(
            publishable_key_id(&format!("pv_sand_{secret}/{id_part}")),
            None
        );
        assert_eq!(publishable_key_id(&format!("pk_sand_{secret}")), None);
        assert_eq!(publishable_key_id("pk_sand_x/not-base62!"), None);
        assert!(publishable_key_prefix(&TenantEnvironmentEnum::Production).starts_with("pk_prod_"));
    }
}
