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
        let insertable = config.to_row(&self.crypt_key)?;

        let mut conn = self.get_conn().await?;

        let row = insertable
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        ProviderConfig::from_row(&self.crypt_key, row)
    }

    async fn find_provider_config(
        &self,
        provider: InvoicingProviderEnum,
        tenant_id: Uuid,
    ) -> StoreResult<ProviderConfig> {
        let mut conn = self.get_conn().await?;

        let row = diesel_models::configs::ProviderConfig::find_provider_config(
            &mut conn,
            tenant_id,
            provider.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        ProviderConfig::from_row(&self.crypt_key, row)
    }
}
