use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use cached::proc_macro::cached;
use common_domain::ids::{ConnectorId, TenantId};
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use hubspot_client::client::HubspotClient;
use hubspot_client::properties::PropertiesApi;
use itertools::Itertools;
use meteroid_oauth::model::OauthProvider;
use meteroid_store::domain::connectors::ProviderSensitiveData;
use meteroid_store::domain::pgmq::{HubspotSyncRequestEvent, PgmqMessage};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use meteroid_store::{Store, StoreResult};
use secrecy::SecretString;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct HubspotSync {
    pub(crate) store: Arc<Store>,
    pub(crate) client: Arc<HubspotClient>,
}

impl HubspotSync {
    async fn get_connected_tenants(
        &self,
        events: &[(HubspotSyncRequestEvent, MessageId)],
    ) -> PgmqResult<Vec<ConnectedTenant>> {
        let by_tenant = events.iter().chunk_by(|x| x.0.tenant_id());

        let mut tasks = vec![];
        for (tenant_id, _) in &by_tenant {
            let store = self.store.clone();
            tasks.push(tokio::spawn(async move {
                let access_token =
                    get_hubspot_access_token_cached(store.as_ref(), tenant_id).await?;
                Ok::<_, Report<PgmqError>>((tenant_id, access_token))
            }));
        }

        let mut connected_tenants = vec![];
        for task in tasks {
            match task.await {
                Ok(Ok((tenant_id, Some((connector_id, access_token))))) => {
                    connected_tenants.push(ConnectedTenant {
                        connector_id,
                        tenant_id,
                        access_token,
                        events: by_tenant
                            .into_iter()
                            .find_map(|(tenant, group)| {
                                if tenant == tenant_id {
                                    Some(group.cloned().collect())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(vec![]),
                    });
                }
                Ok(Ok((tenant_id, None))) => {
                    log::info!("No hubspot connector found for tenant {}", tenant_id);
                }
                Ok(Err(e)) => {
                    log::warn!("Failed to get access token: {:?}", e);
                }
                Err(e) => {
                    log::warn!("Task failed: {:?}", e);
                }
            }
        }

        Ok(connected_tenants)
    }

    fn convert_to_events(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<Vec<(HubspotSyncRequestEvent, MessageId)>> {
        msgs.iter()
            .map(|msg| {
                let evt: StoreResult<HubspotSyncRequestEvent> = msg.try_into();
                evt.map(|evt| (evt, msg.msg_id))
            })
            .collect::<StoreResult<Vec<_>>>()
            .change_context(PgmqError::HandleMessages)
    }

    async fn sync_connected_tenant(&self, tenant: ConnectedTenant) -> PgmqResult<Vec<MessageId>> {
        let mut to_init_props = vec![];
        let mut customers_to_sync = vec![];
        let mut subscriptions_to_sync = vec![];

        for (evt, msg) in tenant.events {
            match evt {
                HubspotSyncRequestEvent::InitProperties { .. } => {
                    to_init_props.push((evt, msg));
                }
                HubspotSyncRequestEvent::Customer { .. } => {
                    customers_to_sync.push((evt, msg));
                }
                HubspotSyncRequestEvent::Subscription { .. } => {
                    subscriptions_to_sync.push((evt, msg));
                }
            }
        }

        if !to_init_props.is_empty() {
            self.client
                .init_meteroid_properties(&tenant.access_token)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }

        todo!()
    }
}

#[async_trait::async_trait]
impl PgmqHandler for HubspotSync {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let events = self.convert_to_events(msgs)?;
        let connected_tenants = self.get_connected_tenants(&events).await?;

        // messages belonging to not connected tenants should be marked as succeeded
        let mut success_msg_ids = events
            .iter()
            .filter_map(|(evt, msg_id)| {
                if connected_tenants
                    .iter()
                    .any(|connected| connected.tenant_id == evt.tenant_id())
                {
                    None
                } else {
                    Some(*msg_id)
                }
            })
            .collect::<Vec<_>>();

        if connected_tenants.is_empty() {
            return Ok(success_msg_ids);
        }

        let tasks = connected_tenants
            .into_iter()
            .map(|connected| {
                tokio::spawn({
                    let value = self.clone();
                    async move { value.sync_connected_tenant(connected).await }
                })
            })
            .collect::<Vec<_>>();

        for task in tasks {
            match task.await {
                Ok(Ok(ids)) => {
                    success_msg_ids.extend(ids);
                }
                Ok(Err(e)) => {
                    log::warn!("Failed to sync connected token: {:?}", e);
                }
                Err(e) => {
                    log::warn!("Sync task failed: {:?}", e);
                }
            }
        }

        Ok(success_msg_ids)
    }
}

#[allow(dead_code)]
struct ConnectedTenant {
    connector_id: ConnectorId,
    tenant_id: TenantId,
    access_token: SecretString,
    events: Vec<(HubspotSyncRequestEvent, MessageId)>,
}

// todo we should use moka cache with per item ttl instead (the ttl should be expires_in)
#[cached(
    result = true,
    size = 100,
    time = 300, // 5 min, hubspot access token currently expires_in 30 mins
    key = "TenantId",
    convert = r#"{ tenant_id }"#,
    sync_writes = "default"
)]
async fn get_hubspot_access_token_cached(
    store: &Store,
    tenant_id: TenantId,
) -> PgmqResult<Option<(ConnectorId, SecretString)>> {
    let connector = store
        .get_hubspot_connector(tenant_id)
        .await
        .change_context(PgmqError::HandleMessages)?;

    if let Some(connector) = connector {
        if let Some(ProviderSensitiveData::Hubspot(data)) = connector.sensitive {
            let access_token = store
                .oauth_exchange_refresh_token(
                    OauthProvider::Hubspot,
                    SecretString::new(data.refresh_token),
                )
                .await
                .change_context(PgmqError::HandleMessages)?;

            return Ok(Some((connector.id, access_token)));
        } else {
            log::warn!(
                "Missing or invalid sensitive data for hubspot connector {}",
                connector.id
            );
        }
    } else {
        log::info!("No hubspot connector found for tenant {}", tenant_id);
    }

    Ok(None)
}
