use crate::domain::connectors::{
    Connector, ConnectorAccessToken, ConnectorMeta, ConnectorNew, HubspotPublicData,
    HubspotSensitiveData, PennylanePublicData, PennylaneSensitiveData, ProviderData,
    ProviderSensitiveData, StripePublicData, StripeSensitiveData,
};
use crate::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
use crate::domain::oauth::{OauthConnected, OauthConnection, OauthVerifierData};
use crate::domain::pgmq::{HubspotSyncRequestEvent, PgmqMessageNew, PgmqQueue};
use crate::errors::StoreError;
use crate::repositories::oauth::OauthInterface;
use crate::{Store, StoreResult};
use chrono::Utc;
use common_domain::ids::{ConnectorId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::connectors::{ConnectorRow, ConnectorRowNew, ConnectorRowPatch};
use diesel_models::query::pgmq;
use error_stack::{Report, bail};
use meteroid_oauth::model::OauthProvider;
use secrecy::{ExposeSecret, SecretString};

#[async_trait::async_trait]
pub trait ConnectorsInterface {
    async fn list_connectors(
        &self,
        connector_type_filter: Option<ConnectorTypeEnum>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Connector>>;

    async fn delete_connector(&self, id: ConnectorId, tenant_id: TenantId) -> StoreResult<()>;

    async fn connect_stripe(
        &self,
        tenant_id: TenantId,
        alias: String,
        publishable_key: String,
        stripe_data: StripeSensitiveData,
        stripe_account_id: String,
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

    async fn get_pennylane_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>>;

    async fn get_pennylane_connector_access_token(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Option<ConnectorAccessToken>>;
}

#[async_trait::async_trait]
impl ConnectorsInterface for Store {
    async fn list_connectors(
        &self,
        connector_type_filter: Option<ConnectorTypeEnum>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<Connector>> {
        let mut conn = self.get_conn().await?;

        let rows = ConnectorRow::list_connectors(
            &mut conn,
            tenant_id,
            connector_type_filter.map(Into::into),
            None,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let connectors = rows
            .into_iter()
            .map(|x| Connector::from_row(&self.settings.crypt_key, x))
            .collect::<StoreResult<Vec<_>>>()?;

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
        stripe_account_id: String,
    ) -> StoreResult<ConnectorMeta> {
        // then insert
        let mut conn = self.get_conn().await?;

        let row: ConnectorRowNew = ConnectorNew {
            tenant_id,
            alias,
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stripe,
            data: Some(ProviderData::Stripe(StripePublicData {
                api_publishable_key: publishable_key,
                account_id: stripe_account_id,
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
        get_connector(
            self,
            tenant_id,
            ConnectorTypeEnum::Crm,
            ConnectorProviderEnum::Hubspot,
        )
        .await
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
            data: Some(Some(ProviderData::Hubspot(data).try_into()?)),
            sensitive: None,
        };

        let row = patch
            .patch(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Connector::from_row(&self.settings.crypt_key, row)
    }

    async fn get_pennylane_connector(&self, tenant_id: TenantId) -> StoreResult<Option<Connector>> {
        get_connector(
            self,
            tenant_id,
            ConnectorTypeEnum::Accounting,
            ConnectorProviderEnum::Pennylane,
        )
        .await
    }

    async fn get_pennylane_connector_access_token(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Option<ConnectorAccessToken>> {
        let connector = self.get_pennylane_connector(tenant_id).await?;

        // todo handle concurrent refreshes + refresh_token expiry (90 days)
        if let Some(connector) = connector
            && let Some(ProviderSensitiveData::Pennylane(sensitive)) = connector.sensitive
            && let Some(ProviderData::Pennylane(data)) = connector.data
        {
            let is_expired = sensitive
                .expires_at
                .as_ref()
                .map(|exp| exp <= &(Utc::now() + chrono::Duration::minutes(30)))
                .unwrap_or(false);

            let auth_token = if is_expired {
                let tokens = self
                    .oauth_exchange_refresh_token(
                        OauthProvider::Pennylane,
                        SecretString::new(sensitive.refresh_token),
                    )
                    .await?;

                let sensitive = PennylaneSensitiveData::try_from(tokens)?;

                let access_token = ConnectorAccessToken {
                    connector_id: connector.id,
                    external_company_id: data.external_company_id,
                    access_token: SecretString::new(sensitive.access_token.clone()),
                    expires_at: sensitive.expires_at,
                };

                let new_sensitive = ProviderSensitiveData::Pennylane(sensitive)
                    .encrypt(&self.settings.crypt_key)?;

                let patch = ConnectorRowPatch {
                    id: connector.id,
                    data: None,
                    sensitive: Some(Some(new_sensitive)),
                };

                let mut conn = self.get_conn().await?;
                patch
                    .patch(&mut conn, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                access_token
            } else {
                ConnectorAccessToken {
                    connector_id: connector.id,
                    external_company_id: data.external_company_id,
                    access_token: SecretString::new(sensitive.access_token),
                    expires_at: sensitive.expires_at,
                }
            };

            return Ok(Some(auth_token));
        }

        Ok(None)
    }
}

async fn connect_hubspot(
    store: &Store,
    oauth_code: SecretString,
    oauth_state: SecretString,
) -> StoreResult<OauthConnected> {
    let OauthConnection {
        user,
        tokens,
        verifier_data,
    } = store
        .oauth_exchange_code(OauthProvider::Hubspot, oauth_code, oauth_state)
        .await?;

    let crm_data = match verifier_data {
        OauthVerifierData::ConnectHubspot(data) => data,
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
        data: Some(ProviderData::Hubspot(HubspotPublicData {
            auto_sync: crm_data.auto_sync,
            external_company_id: user.company_id,
        })),
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
    })
}

async fn connect_pennylane(
    store: &Store,
    oauth_code: SecretString,
    oauth_state: SecretString,
) -> StoreResult<OauthConnected> {
    let OauthConnection {
        user,
        tokens,
        verifier_data,
    } = store
        .oauth_exchange_code(OauthProvider::Pennylane, oauth_code, oauth_state)
        .await?;

    let crm_data = match verifier_data {
        OauthVerifierData::ConnectPennylane(data) => data,
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
        data: Some(ProviderData::Pennylane(PennylanePublicData {
            external_company_id: user.company_id,
        })),
        sensitive: Some(ProviderSensitiveData::Pennylane(PennylaneSensitiveData {
            refresh_token: refresh_token.expose_secret().to_owned(),
            access_token: tokens.access_token.expose_secret().to_owned(),
            expires_at: tokens.expires_in.map(|duration| Utc::now() + duration),
        })),
    }
    .to_row(&store.settings.crypt_key)?;

    let inserted = row
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    Ok(OauthConnected {
        connector: inserted.into(),
    })
}

async fn get_connector(
    store: &Store,
    tenant_id: TenantId,
    connector_type: ConnectorTypeEnum,
    connector_provider: ConnectorProviderEnum,
) -> StoreResult<Option<Connector>> {
    let mut conn = store.get_conn().await?;
    let row = ConnectorRow::list_connectors(
        &mut conn,
        tenant_id,
        Some(connector_type.into()),
        Some(connector_provider.into()),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?
    .into_iter()
    .next();

    row.map(|row| Connector::from_row(&store.settings.crypt_key, row))
        .transpose()
}
