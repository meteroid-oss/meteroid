use crate::domain::connectors::{
    Connector, ConnectorMeta, ConnectorNew, HubspotPublicData, HubspotSensitiveData,
    PennylaneSensitiveData, ProviderData, ProviderSensitiveData, StripePublicData,
    StripeSensitiveData,
};
use crate::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use crate::domain::oauth::{OauthConnected, OauthTokens, OauthVerifierData};
use crate::domain::pgmq::{HubspotSyncRequestEvent, PgmqMessageNew, PgmqQueue};
use crate::errors::StoreError;
use crate::repositories::oauth::OauthInterface;
use crate::{Store, StoreResult};
use common_domain::ids::{ConnectorId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::connectors::{ConnectorRow, ConnectorRowNew, ConnectorRowPatch};
use diesel_models::query::pgmq;
use error_stack::{Report, bail};
use meteroid_oauth::model::OauthProvider;
use secrecy::{ExposeSecret, SecretString};
use stripe_client::accounts::AccountsApi;

#[async_trait::async_trait]
pub trait ConnectorsInterface {
    async fn list_connectors(
        &self,
        connector_type_filter: Option<ConnectorTypeEnum>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ConnectorMeta>>;

    async fn delete_connector(&self, id: ConnectorId, tenant_id: TenantId) -> StoreResult<()>;

    async fn connect_stripe(
        &self,
        tenant_id: TenantId,
        alias: String,
        publishable_key: String,
        stripe_data: StripeSensitiveData,
    ) -> StoreResult<ConnectorMeta>;

    async fn get_connector_with_data(
        &self,
        id: ConnectorId,
        tenant_id: TenantId,
    ) -> StoreResult<Connector>;
    async fn get_connector_with_data_by_alias(
        &self,
        alias: String,
        tenant_id: TenantId,
    ) -> StoreResult<Connector>;

    async fn connect_oauth(
        &self,
        provider: OauthProvider,
        oauth_code: SecretString,
        oauth_state: SecretString,
    ) -> StoreResult<OauthConnected>;

    async fn get_hubspot_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>>;

    async fn update_hubspot_connector(
        &self,
        connector_id: ConnectorId,
        tenant_id: TenantId,
        data: HubspotPublicData,
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
            None,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let connectors = rows.into_iter().map(Into::into).collect();

        Ok(connectors)
    }

    async fn delete_connector(&self, id: ConnectorId, tenant_id: TenantId) -> StoreResult<()> {
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
        publishable_key: String,
        stripe_data: StripeSensitiveData,
    ) -> StoreResult<ConnectorMeta> {
        // we test with the account api, and fail if we cannot reach it
        let secret = &SecretString::new(stripe_data.api_secret_key.clone());
        let account = self
            .stripe
            .get_account(secret)
            .await
            .map_err(|err| Report::new(err).change_context(StoreError::PaymentProviderError))?;

        // then insert
        let mut conn = self.get_conn().await?;

        let row: ConnectorRowNew = ConnectorNew {
            tenant_id,
            alias,
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stripe,
            data: Some(ProviderData::Stripe(StripePublicData {
                api_publishable_key: publishable_key,
                account_id: account.id,
            })),
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
        id: ConnectorId,
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

    async fn connect_oauth(
        &self,
        provider: OauthProvider,
        oauth_code: SecretString,
        oauth_state: SecretString,
    ) -> StoreResult<OauthConnected> {
        match provider {
            OauthProvider::Hubspot => connect_hubspot(self, oauth_code, oauth_state).await,
            OauthProvider::Pennylane => connect_pennylane(self, oauth_code, oauth_state).await,
            OauthProvider::Google => {
                bail!(StoreError::OauthError(
                    "google not supported as a connector".to_string()
                ));
            }
        }
    }

    async fn get_hubspot_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>> {
        let mut conn = self.get_conn().await?;
        let row = ConnectorRow::list_connectors(
            &mut conn,
            tenant_id,
            Some(ConnectorTypeEnum::Crm.into()),
            Some(ConnectorProviderEnum::Hubspot.into()),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .next();

        row.map(|row| Connector::from_row(&self.settings.crypt_key, row))
            .transpose()
    }

    async fn update_hubspot_connector(
        &self,
        connector_id: ConnectorId,
        tenant_id: TenantId,
        data: HubspotPublicData,
    ) -> StoreResult<Connector> {
        let mut conn = self.get_conn().await?;

        let patch = ConnectorRowPatch {
            id: connector_id,
            data: Some(Some(data.try_into()?)),
        };

        let row = patch
            .patch(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }
}

async fn connect_hubspot(
    store: &Store,
    oauth_code: SecretString,
    oauth_state: SecretString,
) -> StoreResult<OauthConnected> {
    let OauthTokens {
        tokens,
        verifier_data,
    } = store
        .oauth_exchange_code(OauthProvider::Hubspot, oauth_code, oauth_state)
        .await?;

    let crm_data = match verifier_data {
        OauthVerifierData::Connect(data) => data,
        _ => {
            bail!(StoreError::OauthError("Invalid verifier data".to_string(),))
        }
    };

    let refresh_token = tokens
        .refresh_token
        .ok_or_else(|| StoreError::OauthError("Missing refresh token".to_string()))?;

    let mut conn = store.get_conn().await?;

    let row: ConnectorRowNew = ConnectorNew {
        tenant_id: crm_data.tenant_id,
        alias: "hubspot".to_owned(),
        connector_type: ConnectorTypeEnum::Crm,
        provider: ConnectorProviderEnum::Hubspot,
        data: None,
        sensitive: Some(ProviderSensitiveData::Hubspot(HubspotSensitiveData {
            refresh_token: refresh_token.expose_secret().to_owned(),
        })),
    }
    .to_row(&store.settings.crypt_key)?;

    let res = store
        .transaction(|tx| {
            async move {
                let inserted = row
                    .insert(&mut conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let msg: PgmqMessageNew =
                    HubspotSyncRequestEvent::CustomProperties(inserted.tenant_id).try_into()?;

                pgmq::send_batch(tx, PgmqQueue::HubspotSync.as_str(), &[msg.into()])
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                Ok(inserted)
            }
            .scope_boxed()
        })
        .await?;

    Ok(OauthConnected {
        connector: res.into(),
        referer: crm_data.referer,
    })
}

async fn connect_pennylane(
    store: &Store,
    oauth_code: SecretString,
    oauth_state: SecretString,
) -> StoreResult<OauthConnected> {
    let OauthTokens {
        tokens,
        verifier_data,
    } = store
        .oauth_exchange_code(OauthProvider::Pennylane, oauth_code, oauth_state)
        .await?;

    let crm_data = match verifier_data {
        OauthVerifierData::Connect(data) => data,
        _ => {
            bail!(StoreError::OauthError("Invalid verifier data".to_string(),))
        }
    };

    let refresh_token = tokens
        .refresh_token
        .ok_or_else(|| StoreError::OauthError("Missing refresh token".to_string()))?;

    let mut conn = store.get_conn().await?;

    let row: ConnectorRowNew = ConnectorNew {
        tenant_id: crm_data.tenant_id,
        alias: "pennylane".to_owned(),
        connector_type: ConnectorTypeEnum::Accounting,
        provider: ConnectorProviderEnum::Pennylane,
        data: None,
        sensitive: Some(ProviderSensitiveData::Pennylane(PennylaneSensitiveData {
            refresh_token: refresh_token.expose_secret().to_owned(),
        })),
    }
    .to_row(&store.settings.crypt_key)?;

    let inserted = row
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    Ok(OauthConnected {
        connector: inserted.into(),
        referer: crm_data.referer,
    })
}
