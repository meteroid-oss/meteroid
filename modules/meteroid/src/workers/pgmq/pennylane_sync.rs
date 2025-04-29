use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use cached::proc_macro::cached;
use common_domain::ids::{BankAccountId, ConnectorId, TenantId};
use common_domain::pgmq::MessageId;
use common_logging::unwrapper::UnwrapLogger;
use error_stack::{ResultExt, report};
use itertools::Itertools;
use meteroid_oauth::model::{OauthAccessToken, OauthProvider};
use meteroid_store::domain::connectors::{Connector, ProviderSensitiveData};
use meteroid_store::domain::outbox_event::{CustomerEvent, InvoiceEvent};
use meteroid_store::domain::pgmq::{
    PennylaneSyncCustomer, PennylaneSyncInvoice, PennylaneSyncRequestEvent, PgmqMessage,
};
use meteroid_store::domain::{Address, BankAccountFormat, ConnectorProviderEnum};
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface};
use meteroid_store::{Store, StoreResult};
use moka::Expiry;
use moka::future::Cache;
use pennylane_client::client::PennylaneClient;
use pennylane_client::customers::{BillingAddress, CustomersApi, NewCompany, UpdateCompany};
use secrecy::SecretString;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// todo extract out common and reuse in hubspot and pennylane

#[derive(Clone)]
pub(crate) struct PennylaneSync {
    pub(crate) store: Arc<Store>,
    pub(crate) client: Arc<PennylaneClient>,
    pub(crate) token_cache: Cache<ConnectorId, OauthAccessToken>,
}

impl PennylaneSync {
    pub(crate) fn new(store: Arc<Store>, client: Arc<PennylaneClient>) -> Self {
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
        events: Vec<(PennylaneSyncRequestEvent, MessageId)>,
    ) -> PgmqResult<(Vec<ConnectedTenant>, Vec<MessageId>)> {
        let by_tenant = events.into_iter().chunk_by(|(evt, _)| evt.tenant_id());

        let mut tasks = vec![];

        for (tenant_id, chunk) in &by_tenant {
            let store = self.store.clone();
            let cache = self.token_cache.clone();
            let events: Vec<(PennylaneSyncRequestEvent, MessageId)> = chunk.collect_vec();

            tasks.push((
                tenant_id,
                events,
                tokio::spawn(async move {
                    get_pennylane_connector(store.as_ref(), cache, tenant_id).await
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
                    log::info!("No pennylane connector found for tenant {tenant_id}");
                }
                Ok(Err(e)) => {
                    log::warn!("Failed to get access token for tenant {tenant_id}: {:?}", e);
                }
                Err(e) => {
                    log::warn!("Task failed: {:?}", e);
                }
            }
        }

        Ok((connected_tenants, orphan_msg_ids))
    }

    fn convert_to_events(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<Vec<(PennylaneSyncRequestEvent, MessageId)>> {
        msgs.iter()
            .map(|msg| {
                let evt: StoreResult<PennylaneSyncRequestEvent> = msg.try_into();
                evt.map(|evt| (evt, msg.msg_id))
            })
            .collect::<StoreResult<Vec<_>>>()
            .change_context(PgmqError::HandleMessages)
    }

    async fn sync_connected_tenant(&self, conn: ConnectedTenant) -> PgmqResult<Vec<MessageId>> {
        let mut customers_to_sync = vec![];
        let mut invoices_to_sync = vec![];
        let mut customer_outbox_to_sync = vec![];
        let mut invoice_outbox_to_sync = vec![];

        for (evt, msg) in conn.events {
            match evt {
                PennylaneSyncRequestEvent::Customer(data) => {
                    customers_to_sync.push((data, msg));
                }
                PennylaneSyncRequestEvent::Invoice(data) => {
                    invoices_to_sync.push((data, msg));
                }
                PennylaneSyncRequestEvent::CustomerOutbox(data) => {
                    customer_outbox_to_sync.push((data, msg));
                }
                PennylaneSyncRequestEvent::InvoiceOutbox(data) => {
                    invoice_outbox_to_sync.push((data, msg));
                }
            }
        }

        let conn = conn.connector;

        let succeeded_customers = self
            .sync_customers(&conn, customers_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync customers: {:?}", e));
        let succeeded_invoices = self
            .sync_invoices(&conn, invoices_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync invoices: {:?}", e));
        let succeeded_cus_outbox = self
            .sync_customer_outbox(&conn, customer_outbox_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync customer outbox events: {:?}", e));
        let succeeded_inv_outbox = self
            .sync_invoice_outbox(&conn, invoice_outbox_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync invoice outbox events: {:?}", e));

        Ok(succeeded_customers
            .into_iter()
            .chain(succeeded_invoices)
            .chain(succeeded_cus_outbox)
            .chain(succeeded_inv_outbox)
            .collect_vec())
    }

    async fn sync_customers(
        &self,
        conn: &PennylaneConnector,
        customer_reqs: Vec<(Box<PennylaneSyncCustomer>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if customer_reqs.is_empty() {
            return Ok(Vec::new());
        }

        let ids = customer_reqs
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
                customer_reqs.iter().filter_map(move |(sync_cus, msg_id)| {
                    if sync_cus.id == cus_id {
                        Some((Box::new(event.clone()), *msg_id))
                    } else {
                        None
                    }
                })
            })
            .collect_vec();

        let succeeded = self.sync_customer_outbox(conn, customer_events).await?;

        Ok(succeeded)
    }

    async fn sync_invoices(
        &self,
        conn: &PennylaneConnector,
        invoice_reqs: Vec<(Box<PennylaneSyncInvoice>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if invoice_reqs.is_empty() {
            return Ok(Vec::new());
        }

        let ids = invoice_reqs
            .iter()
            .map(|(inv, _)| inv.id)
            .unique()
            .collect_vec();

        let invoices = self
            .store
            .list_invoices_by_ids(ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let invoice_events = invoices
            .into_iter()
            .flat_map(|invoice| {
                let inv_id = invoice.id;
                let event: InvoiceEvent = invoice.into();
                invoice_reqs.iter().filter_map(move |(sync_inv, msg_id)| {
                    if sync_inv.id == inv_id {
                        Some((Box::new(event.clone()), *msg_id))
                    } else {
                        None
                    }
                })
            })
            .collect_vec();

        let succeeded = self.sync_invoice_outbox(conn, invoice_events).await?;

        Ok(succeeded)
    }

    async fn sync_customer_outbox(
        &self,
        conn: &PennylaneConnector,
        outboxes: Vec<(Box<CustomerEvent>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if outboxes.is_empty() {
            return Ok(Vec::new());
        }

        let mut succeeded_msgs = vec![];

        for (event, msg_id) in outboxes {
            let customer_id = event.customer_id;
            let billing_iban = if let Some(id) = event.bank_account_id {
                self.get_billing_iban(id, conn.tenant_id)
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };

            let res = match event.get_pennylane_id(conn.id) {
                Some(pennylane_id) => {
                    let company = Self::convert_to_update_company(*event, billing_iban);

                    self.client
                        .update_company_customer(pennylane_id, company, &conn.access_token)
                        .await
                }
                None => {
                    let company = Self::convert_to_new_company(*event, billing_iban);

                    self.client
                        .create_company_customer(company, &conn.access_token)
                        .await
                }
            };

            match res {
                Ok(res) => {
                    self.store
                        .patch_customer_conn_meta(
                            customer_id,
                            conn.id,
                            ConnectorProviderEnum::Pennylane,
                            res.id.to_string().as_str(),
                        )
                        .await
                        .change_context(PgmqError::HandleMessages)?;

                    log::info!("Customer {customer_id} synced to pennylane");

                    succeeded_msgs.push(msg_id);
                }
                Err(e) => {
                    log::warn!("Failed to create/update customer in pennylane: {:?}", e);
                    let status_code = e.status_code();

                    if status_code.is_some_and(|x| x < 500 && x != 429) {
                        succeeded_msgs.push(msg_id);
                    }
                }
            }
        }

        Ok(succeeded_msgs)
    }

    async fn sync_invoice_outbox(
        &self,
        _conn: &PennylaneConnector,
        outboxes: Vec<(Box<InvoiceEvent>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if outboxes.is_empty() {
            return Ok(Vec::new());
        }

        // todo implement me
        Ok(outboxes.into_iter().map(|(_, msg_id)| msg_id).collect_vec())
    }

    fn convert_to_billing_address(ba: Option<&Address>) -> BillingAddress {
        let address = ba.and_then(|x| x.line1.clone()).unwrap_or_default();

        let postal_code = ba.and_then(|x| x.zip_code.clone()).unwrap_or_default();
        let city = ba.and_then(|x| x.city.clone()).unwrap_or_default();

        // todo check if country is 2 letters
        let country_alpha2 = ba.and_then(|x| x.country.clone()).unwrap_or_default();

        BillingAddress {
            address,
            postal_code,
            city,
            country_alpha2,
        }
    }

    fn convert_to_new_company(event: CustomerEvent, billing_iban: Option<String>) -> NewCompany {
        let billing_address = Self::convert_to_billing_address(event.billing_address.as_ref());

        NewCompany {
            name: event.name,
            billing_address,
            billing_email: event.billing_email,
            phone: event.phone.clone(),
            external_reference: event.customer_id.as_proto(),
            vat_number: event.vat_number,
            emails: event.invoicing_emails,
            billing_iban,
        }
    }

    fn convert_to_update_company(
        event: CustomerEvent,
        billing_iban: Option<String>,
    ) -> UpdateCompany {
        let billing_address = Self::convert_to_billing_address(event.billing_address.as_ref());

        UpdateCompany {
            name: event.name,
            billing_address,
            billing_email: event.billing_email.clone(),
            phone: event.phone.clone(),
            external_reference: event.customer_id.as_proto(),
            vat_number: event.vat_number.clone(),
            emails: event.invoicing_emails,
            billing_iban,
        }
    }

    async fn get_billing_iban(
        &self,
        bank_account_id: BankAccountId,
        tenant_id: TenantId,
    ) -> PgmqResult<Option<String>> {
        let bank_account = self
            .store
            .get_bank_account_by_id(bank_account_id, tenant_id)
            .await
            .change_context(PgmqError::HandleMessages)?;

        if bank_account.format == BankAccountFormat::IbanBicSwift {
            Ok(Some(bank_account.account_numbers))
        } else {
            Ok(None)
        }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for PennylaneSync {
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
                    log::warn!("Failed to sync connected tenant: {:?}", e);
                }
                Err(e) => {
                    log::warn!("Sync task failed: {:?}", e);
                }
            }
        }

        Ok(success_msg_ids)
    }
}

async fn get_pennylane_connector(
    store: &Store,
    cache: Cache<ConnectorId, OauthAccessToken>,
    tenant_id: TenantId,
) -> PgmqResult<Option<PennylaneConnector>> {
    let connector = get_connector_cached(store, tenant_id).await?;

    if let Some(connector) = connector {
        if let Some(ProviderSensitiveData::Pennylane(data)) = connector.sensitive {
            let refresh_token = SecretString::new(data.refresh_token);

            let token = cache
                .try_get_with(
                    connector.id,
                    store.oauth_exchange_refresh_token(OauthProvider::Pennylane, refresh_token),
                )
                .await
                .map_err(|x| {
                    if let Some(e) = Arc::into_inner(x) {
                        e.change_context(PgmqError::HandleMessages)
                    } else {
                        report!(PgmqError::HandleMessages)
                    }
                })?;

            return Ok(Some(PennylaneConnector {
                id: connector.id,
                tenant_id: connector.tenant_id,
                access_token: token.value,
            }));
        } else {
            log::warn!("Pennylane connector has missing/illegal sensitive data");
        }
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
        .get_pennylane_connector(tenant_id)
        .await
        .change_context(PgmqError::HandleMessages)
}

struct PennylaneConnector {
    id: ConnectorId,
    tenant_id: TenantId,
    access_token: SecretString,
}

#[allow(dead_code)]
struct ConnectedTenant {
    connector: PennylaneConnector,
    events: Vec<(PennylaneSyncRequestEvent, MessageId)>,
}

struct OauthAccessTokenExpiry;

impl Expiry<ConnectorId, OauthAccessToken> for OauthAccessTokenExpiry {
    fn expire_after_create(
        &self,
        _key: &ConnectorId,
        value: &OauthAccessToken,
        _created_at: Instant,
    ) -> Option<Duration> {
        value
            .expires_in
            .or(Some(Duration::from_secs(86400))) // 1 day by default
            .map(|x| x - Duration::from_secs(60)) // expire 1 minute earlier
    }
}
