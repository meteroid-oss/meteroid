use error_stack::Report;
use uuid::Uuid;

use crate::domain::configs::{ProviderConfig, ProviderConfigNew};
use crate::domain::enums::InvoicingProviderEnum;
use crate::errors::StoreError;
use crate::{Store, StoreResult};

#[async_trait::async_trait]
pub trait ConfigsInterface {
    async fn insert_provider_config(
        &self,
        config: ProviderConfigNew,
    ) -> StoreResult<ProviderConfig>;

    async fn find_provider_config(
        &self,
        provider: InvoicingProviderEnum,
        tenant_id: Uuid,
    ) -> StoreResult<ProviderConfig>;
}

#[async_trait::async_trait]
impl ConfigsInterface for Store {
    async fn insert_provider_config(
        &self,
        config: ProviderConfigNew,
    ) -> StoreResult<ProviderConfig> {
        let insertable: diesel_models::configs::ProviderConfigNew =
            config.encrypted(&self.crypt_key)?.try_into()?;

        let mut conn = self.get_conn().await?;

        let enc: ProviderConfig = insertable
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()?;

        enc.decrypted(&self.crypt_key)
    }

    async fn find_provider_config(
        &self,
        provider: InvoicingProviderEnum,
        tenant_id: Uuid,
    ) -> StoreResult<ProviderConfig> {
        let mut conn = self.get_conn().await?;

        let enc: ProviderConfig = diesel_models::configs::ProviderConfig::find_provider_config(
            &mut conn,
            tenant_id,
            provider.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .try_into()?;

        enc.decrypted(&self.crypt_key)
    }
}
