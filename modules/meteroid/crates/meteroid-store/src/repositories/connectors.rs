use crate::domain::connectors::{
    Connector, ConnectorMeta, ConnectorNew, ProviderSensitiveData, StripeSensitiveData,
};
use crate::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use common_domain::ids::TenantId;
use diesel_models::connectors::{ConnectorRow, ConnectorRowNew};
use error_stack::Report;
use secrecy::SecretString;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait ConnectorsInterface {
    async fn list_connectors(
        &self,
        connector_type_filter: Option<ConnectorTypeEnum>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ConnectorMeta>>;

    async fn delete_connector(&self, id: Uuid, tenant_id: TenantId) -> StoreResult<()>;

    async fn connect_stripe(
        &self,
        tenant_id: TenantId,
        alias: String,
        stripe_data: StripeSensitiveData,
    ) -> StoreResult<ConnectorMeta>;

    async fn get_connector_with_data(
        &self,
        id: Uuid,
        tenant_id: TenantId,
    ) -> StoreResult<Connector>;
    async fn get_connector_with_data_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<Connector>;
}

#[async_trait::async_trait]
impl ConnectorsInterface for Store {
    async fn list_connectors(
        &self,
        connector_type_filter: Option<ConnectorTypeEnum>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ConnectorMeta>> {
        let mut conn = self.get_conn().await?;

        let rows = ConnectorRow::list_connectors(
            &mut conn,
            tenant_id,
            connector_type_filter.map(Into::into),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let connectors = rows.into_iter().map(Into::into).collect();

        Ok(connectors)
    }

    async fn delete_connector(&self, id: Uuid, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        ConnectorRow::delete_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(())
    }

    async fn connect_stripe(
        &self,
        tenant_id: TenantId,
        alias: String,
        stripe_data: StripeSensitiveData,
    ) -> StoreResult<ConnectorMeta> {
        // we test with the account api, and fail if we cannot reach it
        let secret = &SecretString::new(stripe_data.api_secret_key.clone());
        self.stripe
            .get_account_id(secret)
            .await
            .map_err(|err| Report::new(err).change_context(StoreError::StripeError))?;

        // then insert
        let mut conn = self.get_conn().await?;

        let row: ConnectorRowNew = ConnectorNew {
            tenant_id,
            alias,
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stripe,
            data: None,
            sensitive: Some(ProviderSensitiveData::Stripe(stripe_data)),
        }
        .to_row(&self.settings.crypt_key)?;

        let res = row
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(res.into())
    }

    async fn get_connector_with_data(
        &self,
        id: Uuid,
        tenant_id: TenantId,
    ) -> StoreResult<Connector> {
        let mut conn = self.get_conn().await?;

        let row = ConnectorRow::get_connector_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }

    async fn get_connector_with_data_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<Connector> {
        let mut conn = self.get_conn().await?;

        let row = ConnectorRow::get_connector_by_alias(&mut conn, alias, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }
}
