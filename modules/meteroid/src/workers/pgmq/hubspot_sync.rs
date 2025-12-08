use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use cached::proc_macro::cached;
use common_domain::ids::{ConnectorId, CustomerId, TenantId};
use common_domain::pgmq::MessageId;
use common_logging::unwrapper::UnwrapLogger;
use error_stack::{Report, ResultExt};
use hubspot_client::associations::AssociationsApi;
use hubspot_client::client::HubspotClient;
use hubspot_client::companies::{CompaniesApi, CompanyAddress, NewCompany};
use hubspot_client::deals::{DealsApi, NewDeal};
use hubspot_client::model::CompanyId;
use hubspot_client::properties::PropertiesApi;
use itertools::Itertools;
use meteroid_oauth::model::{OAuthTokens, OauthProvider};
use meteroid_store::domain::ConnectorProviderEnum;
use meteroid_store::domain::connectors::{
    Connector, HubspotPublicData, ProviderData, ProviderSensitiveData,
};
use meteroid_store::domain::outbox_event::{CustomerEvent, SubscriptionEvent};
use meteroid_store::domain::pgmq::{
    HubspotSyncCustomerDomain, HubspotSyncRequestEvent, HubspotSyncSubscription, PgmqMessage,
};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use meteroid_store::repositories::{CustomersInterface, SubscriptionInterface};
use meteroid_store::{Store, StoreResult};
use moka::Expiry;
use moka::future::Cache;
use secrecy::SecretString;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(crate) struct HubspotSync {
    pub(crate) store: Arc<Store>,
    pub(crate) client: Arc<HubspotClient>,
    pub(crate) token_cache: Cache<ConnectorId, OAuthTokens>,
}

impl HubspotSync {
    pub(crate) fn new(store: Arc<Store>, client: Arc<HubspotClient>) -> Self {
        let token_cache = Cache::builder()
            .expire_after(OauthAccessTokenExpiry)
            .max_capacity(500)
            .build();

        Self {
            store,
            client,
            token_cache,
        }
    }

    async fn get_connected_tenants(
        &self,
        events: Vec<(HubspotSyncRequestEvent, MessageId)>,
    ) -> PgmqResult<(Vec<ConnectedTenant>, Vec<MessageId>)> {
        let by_tenant = events.into_iter().chunk_by(|(evt, _)| evt.tenant_id());

        let mut tasks = vec![];

        for (tenant_id, chunk) in &by_tenant {
            let store = self.store.clone();
            let cache = self.token_cache.clone();
            let events: Vec<(HubspotSyncRequestEvent, MessageId)> = chunk.collect_vec();

            tasks.push((
                tenant_id,
                events,
                tokio::spawn(async move {
                    get_hubspot_connector(store.as_ref(), cache, tenant_id).await
                }),
            ));
        }

        let mut connected_tenants = vec![];

        // messages belonging to not connected tenants
        let mut orphan_msg_ids = vec![];

        for (tenant_id, events, task) in tasks {
            match task.await {
                Ok(Ok(Some(connector))) => {
                    connected_tenants.push(ConnectedTenant { connector, events });
                }
                Ok(Ok(None)) => {
                    orphan_msg_ids.extend(events.into_iter().map(|(_, msg_id)| msg_id));
                    log::info!("No hubspot connector found for tenant {tenant_id}");
                }
                Ok(Err(e)) => {
                    log::warn!("Failed to get access token for tenant {tenant_id}: {e:?}");
                }
                Err(e) => {
                    log::warn!("Task failed: {e:?}");
                }
            }
        }

        Ok((connected_tenants, orphan_msg_ids))
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

    async fn sync_connected_tenant(&self, conn: ConnectedTenant) -> PgmqResult<Vec<MessageId>> {
        let mut custom_props_to_sync = vec![];
        let mut customer_domains_to_sync = vec![];
        let mut subscriptions_to_sync = vec![];
        let mut customer_outbox_to_sync = vec![];
        let mut subscription_outbox_to_sync = vec![];
        let mut ignored_messages = vec![];

        for (evt, msg) in conn.events {
            match evt {
                HubspotSyncRequestEvent::CustomProperties(_) => {
                    custom_props_to_sync.push(msg);
                }
                HubspotSyncRequestEvent::CustomerDomain(data) => {
                    customer_domains_to_sync.push((data, msg));
                }
                HubspotSyncRequestEvent::Subscription(data) => {
                    subscriptions_to_sync.push((data, msg));
                }
                HubspotSyncRequestEvent::CustomerOutbox(data) => {
                    if conn.connector.data.auto_sync {
                        customer_outbox_to_sync.push((data, msg));
                    } else {
                        ignored_messages.push(msg);
                    }
                }
                HubspotSyncRequestEvent::SubscriptionOutbox(data) => {
                    if conn.connector.data.auto_sync {
                        subscription_outbox_to_sync.push((data, msg));
                    } else {
                        ignored_messages.push(msg);
                    }
                }
            }
        }

        let conn = conn.connector;

        let succeeded_props = self
            .sync_custom_props(&conn, custom_props_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to init properties: {e:?}"));
        let succeeded_domains = self
            .sync_customer_domains(&conn, customer_domains_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync customer domains: {e:?}"));
        let succeeded_subscriptions = self
            .sync_subscriptions(&conn, subscriptions_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync subscriptions: {e:?}"));
        let succeeded_cus_outbox = self
            .sync_customer_outbox(&conn, customer_outbox_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync customer outbox events: {e:?}"));
        let succeeded_sub_outbox = self
            .sync_subscription_outbox(&conn, subscription_outbox_to_sync)
            .await
            .unwrap_to_default_warn(|e| {
                format!("Failed to sync subscription outbox events: {e:?}")
            });

        Ok(succeeded_props
            .into_iter()
            .chain(succeeded_domains)
            .chain(succeeded_subscriptions)
            .chain(succeeded_cus_outbox)
            .chain(succeeded_sub_outbox)
            .chain(ignored_messages)
            .collect_vec())
    }

    async fn sync_custom_props(
        &self,
        conn: &HubspotConnector,
        props: Vec<MessageId>,
    ) -> PgmqResult<Vec<MessageId>> {
        if props.is_empty() {
            return Ok(props);
        }

        self.client
            .create_meteroid_properties(&conn.access_token)
            .await
            .change_context(PgmqError::HandleMessages)?;

        Ok(props)
    }

    async fn sync_customer_domains(
        &self,
        conn: &HubspotConnector,
        domains: Vec<(Box<HubspotSyncCustomerDomain>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if domains.is_empty() {
            return Ok(Vec::new());
        }

        let ids = domains
            .iter()
            .map(|(domain, _)| domain.id)
            .unique()
            .collect_vec();

        let customers = self
            .store
            .list_customers_by_ids_global(ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let customer_events = customers
            .into_iter()
            .flat_map(|customer| {
                let cus_id = customer.id;
                let event: CustomerEvent = customer.into();
                domains.iter().filter_map(move |(domain, msg_id)| {
                    if domain.id == cus_id {
                        Some((Box::new(event.clone()), *msg_id))
                    } else {
                        None
                    }
                })
            })
            .collect_vec();

        let succeeded = self.sync_customer_outbox(conn, customer_events).await?;

        let customer_ids = succeeded
            .iter()
            .filter_map(|msg_id| {
                domains.iter().find_map(|(domain, dmsg_id)| {
                    if msg_id == dmsg_id {
                        Some(domain.id)
                    } else {
                        None
                    }
                })
            })
            .collect_vec();

        // enqueue subscriptions for the customers
        self.store
            .sync_customer_subscriptions_to_hubspot(conn.tenant_id, customer_ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        Ok(succeeded)
    }

    async fn sync_subscriptions(
        &self,
        conn: &HubspotConnector,
        subscriptions: Vec<(Box<HubspotSyncSubscription>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if subscriptions.is_empty() {
            return Ok(Vec::new());
        }

        let ids = subscriptions
            .iter()
            .map(|(sub, _)| sub.id)
            .unique()
            .collect_vec();

        let subs = self
            .store
            .list_subscription_by_ids_global(ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let sub_events = subs
            .into_iter()
            .flat_map(|sub| {
                let sub_id = sub.id;
                let event: SubscriptionEvent = sub.into();
                subscriptions.iter().filter_map(move |(sub_msg, msg_id)| {
                    if sub_msg.id == sub_id {
                        Some((Box::new(event.clone()), *msg_id))
                    } else {
                        None
                    }
                })
            })
            .collect_vec();

        self.sync_subscription_outbox(conn, sub_events).await
    }

    async fn sync_customer_outbox(
        &self,
        conn: &HubspotConnector,
        outboxes: Vec<(Box<CustomerEvent>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if outboxes.is_empty() {
            return Ok(Vec::new());
        }

        let new_companies = outboxes
            .iter()
            .map(|(ce, _)| NewCompany {
                customer_id: ce.customer_id,
                name: ce.name.clone(),
                billing_email: ce.billing_email.clone(),
                billing_address: ce.billing_address.clone().map(|a| CompanyAddress {
                    line1: a.line1,
                    line2: a.line2,
                    country: a.country,
                    city: a.city,
                    state: a.state,
                    zip_code: a.zip_code,
                }),
            })
            .collect_vec();

        let response = self
            .client
            .upsert_companies(new_companies, &conn.access_token)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let mut succeeded_msgs = vec![];

        for (customer, msg_id) in &outboxes {
            if let Some(company_id) = response.get_company_id(customer.customer_id) {
                let state_res = self
                    .store
                    .patch_customer_conn_meta(
                        conn.tenant_id,
                        customer.customer_id,
                        conn.id,
                        ConnectorProviderEnum::Hubspot,
                        company_id.0.as_str(),
                        conn.data.external_company_id.as_str(),
                    )
                    .await;

                if let Err(e) = state_res {
                    log::warn!(
                        "Failed to update customer {} hubspot connection metadata in DB: {:?}",
                        customer.customer_id,
                        e
                    );
                } else {
                    log::info!(
                        "Customer {} synced to hubspot company [id={}]",
                        customer.customer_id,
                        company_id.0
                    );
                    succeeded_msgs.push(*msg_id);
                }
            }
        }

        Ok(succeeded_msgs)
    }

    async fn sync_subscription_outbox(
        &self,
        conn: &HubspotConnector,
        outboxes: Vec<(Box<SubscriptionEvent>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if outboxes.is_empty() {
            return Ok(Vec::new());
        }

        let customer_ids = outboxes
            .iter()
            .map(|(event, _)| event.customer_id)
            .unique()
            .collect_vec();

        let customers = self
            .store
            .list_customers_by_ids_global(customer_ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let get_customer_external_id = |customer_id: CustomerId| -> Option<CompanyId> {
            customers
                .iter()
                .find(|customer| customer.id == customer_id)
                .and_then(|customer| {
                    customer
                        .conn_meta
                        .as_ref()
                        .and_then(|meta| meta.hubspot.as_ref())
                        .and_then(|hubspot| hubspot.first())
                        .map(|meta| CompanyId(meta.external_id.clone()))
                })
        };

        let new_deals = outboxes
            .iter()
            .filter_map(|(event, msg_id)| {
                get_customer_external_id(event.customer_id).map(|ext_id| {
                    (
                        NewDeal {
                            subscription_id: event.subscription_id,
                            customer_id: event.customer_id,
                            customer_name: event.customer_name.clone(),
                            plan_name: event.plan_name.clone(),
                            subscription_start_date: event.start_date,
                            subscription_end_date: event.end_date,
                            subscription_currency: event.currency.clone(),
                            subscription_mrr_cents: event.mrr_cents,
                            subscription_status: event.status.as_screaming_snake_case(),
                        },
                        ext_id,
                        *msg_id,
                    )
                })
            })
            .collect_vec();

        let upsert_response = self
            .client
            .upsert_deals(
                new_deals.into_iter().map(|x| x.0).collect_vec(),
                &conn.access_token,
            )
            .await
            .change_context(PgmqError::HandleMessages)?;

        let mut succeeded_msg_ids = vec![];
        let mut succeeded_hs_ids = vec![];

        for (subscription, msg_id) in &outboxes {
            let hs_ids = upsert_response
                .get_deal_id(subscription.subscription_id)
                .and_then(|deal_id| {
                    get_customer_external_id(subscription.customer_id)
                        .map(|company_id| (deal_id, company_id))
                });

            if let Some((deal_id, company_id)) = hs_ids {
                let state_res = self
                    .store
                    .patch_subscription_conn_meta(
                        subscription.subscription_id,
                        conn.id,
                        ConnectorProviderEnum::Hubspot,
                        deal_id.0.as_str(),
                        conn.data.external_company_id.as_str(),
                    )
                    .await;

                if let Err(e) = state_res {
                    log::warn!(
                        "Failed to update subscription {} hubspot connection metadata in DB: {:?}",
                        subscription.subscription_id,
                        e
                    );
                } else {
                    log::info!(
                        "Subscription {} synced to hubspot deal [id={}]",
                        subscription.subscription_id,
                        deal_id.0
                    );
                    succeeded_msg_ids.push(*msg_id);
                    succeeded_hs_ids.push((deal_id, company_id));
                }
            }
        }

        if !succeeded_hs_ids.is_empty() {
            self.client
                .associate_deals_to_companies(succeeded_hs_ids, &conn.access_token)
                .await
                .change_context(PgmqError::HandleMessages)?;
        }

        Ok(succeeded_msg_ids)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for HubspotSync {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let events = self.convert_to_events(msgs)?;
        let (connected_tenants, orphan_msg_ids) = self.get_connected_tenants(events).await?;

        // messages that are not connected to any tenant should be marked as success
        let mut success_msg_ids = orphan_msg_ids;

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
            .collect_vec();

        for task in tasks {
            match task.await {
                Ok(Ok(ids)) => {
                    success_msg_ids.extend(ids);
                }
                Ok(Err(e)) => {
                    log::warn!("Failed to sync connected tenant: {e:?}");
                }
                Err(e) => {
                    log::warn!("Sync task failed: {e:?}");
                }
            }
        }

        Ok(success_msg_ids)
    }
}

#[allow(dead_code)]
struct ConnectedTenant {
    connector: HubspotConnector,
    events: Vec<(HubspotSyncRequestEvent, MessageId)>,
}

async fn get_hubspot_connector(
    store: &Store,
    cache: Cache<ConnectorId, OAuthTokens>,
    tenant_id: TenantId,
) -> PgmqResult<Option<HubspotConnector>> {
    let connector = get_connector_cached(store, tenant_id).await?;

    if let Some(connector) = connector {
        if let (
            Some(ProviderSensitiveData::Hubspot(data)),
            Some(ProviderData::Hubspot(public_data)),
        ) = (connector.sensitive, connector.data)
        {
            let refresh_token = SecretString::from(data.refresh_token.clone());

            let tokens = cache
                .try_get_with(
                    connector.id,
                    store.oauth_exchange_refresh_token(OauthProvider::Hubspot, refresh_token),
                )
                .await
                .map_err(|x| {
                    if let Some(e) = Arc::into_inner(x) {
                        e.change_context(PgmqError::HandleMessages)
                    } else {
                        Report::new(PgmqError::HandleMessages)
                    }
                })?;

            return Ok(Some(HubspotConnector {
                id: connector.id,
                tenant_id: connector.tenant_id,
                data: public_data,
                access_token: tokens.access_token,
            }));
        }
        log::warn!("Misconfigured hubspot connector {}", connector.id);
    } else {
        log::info!("No hubspot connector found for tenant {tenant_id}");
    }

    Ok(None)
}

#[cached(
    result = true,
    size = 100,
    time = 60,
    key = "TenantId",
    convert = r#"{ tenant_id }"#,
    sync_writes = "default"
)]
pub(crate) async fn get_connector_cached(
    store: &Store,
    tenant_id: TenantId,
) -> PgmqResult<Option<Connector>> {
    store
        .get_hubspot_connector(tenant_id)
        .await
        .change_context(PgmqError::HandleMessages)
}

#[derive(Clone)]
struct HubspotConnector {
    id: ConnectorId,
    tenant_id: TenantId,
    data: HubspotPublicData,
    access_token: SecretString,
}

struct OauthAccessTokenExpiry;

impl Expiry<ConnectorId, OAuthTokens> for OauthAccessTokenExpiry {
    fn expire_after_create(
        &self,
        _key: &ConnectorId,
        value: &OAuthTokens,
        _created_at: Instant,
    ) -> Option<Duration> {
        value
            .expires_in
            .or(Some(Duration::from_secs(86400))) // 1 day by default
            .map(|x| x - Duration::from_secs(60)) // expire 1 minute earlier
    }
}
