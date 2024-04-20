use uuid::Uuid;

use crate::domain::api_tokens::ApiToken;
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

    async fn insert_api_token(&self, plan: domain::ApiTokenNew) -> StoreResult<ApiToken>;
}

#[async_trait::async_trait]
impl ApiTokensInterface for Store {
    async fn find_api_tokens_by_tenant_id(
        &self,
        tenant_id: &uuid::Uuid,
    ) -> StoreResult<Vec<ApiToken>> {
        let mut conn = self.get_conn().await?;

        let api_tokens =
            diesel_models::api_tokens::ApiToken::find_by_tenant_id(&mut conn, tenant_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_tokens.into_iter().map(Into::into).collect())
    }

    async fn get_api_token_by_id(&self, id: &Uuid) -> StoreResult<ApiToken> {
        let mut conn = self.get_conn().await?;

        let api_token = diesel_models::api_tokens::ApiToken::find_by_id(&mut conn, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(api_token.into())
    }

    async fn insert_api_token(&self, entity: domain::ApiTokenNew) -> StoreResult<ApiToken> {
        let mut conn = self.get_conn().await?;

        let insertable_entity: diesel_models::api_tokens::ApiTokenNew = entity.into();

        insertable_entity
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }
}
