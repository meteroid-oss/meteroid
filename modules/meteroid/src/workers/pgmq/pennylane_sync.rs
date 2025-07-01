use crate::services::storage::{ObjectStoreService, Prefix};
use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use cached::proc_macro::cached;
use common_domain::ids::{ConnectorId, TenantId};
use common_domain::pgmq::MessageId;
use common_logging::unwrapper::UnwrapLogger;
use error_stack::{ResultExt, report};
use itertools::Itertools;
use meteroid_oauth::model::{OauthAccessToken, OauthProvider};
use meteroid_store::domain::connectors::{Connector, ProviderSensitiveData};
use meteroid_store::domain::outbox_event::CustomerEvent;
use meteroid_store::domain::pgmq::{
    PennylaneSyncCustomer, PennylaneSyncInvoice, PennylaneSyncRequestEvent, PgmqMessage,
};
use meteroid_store::domain::{Address, ConnectorProviderEnum, DetailedInvoice};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface};
use common_utils::decimals::ToUnit;
use meteroid_store::{Store, StoreResult};
use moka::Expiry;
use moka::future::Cache;
use pennylane_client::client::PennylaneClient;
use pennylane_client::customer_invoices::{
    CustomerInvoiceLine, CustomerInvoicesApi, NewCustomerInvoiceImport,
};
use pennylane_client::customers::{BillingAddress, CustomersApi, NewCompany, UpdateCompany};
use pennylane_client::file_attachments::{FileAttachmentsApi, MediaType, NewAttachment};
use rust_decimal::Decimal;
use secrecy::SecretString;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// todo extract out common and reuse in hubspot and pennylane

#[derive(Clone)]
pub(crate) struct PennylaneSync {
    pub(crate) store: Arc<Store>,
    pub(crate) client: Arc<PennylaneClient>,
    pub(crate) token_cache: Cache<ConnectorId, OauthAccessToken>,
    pub(crate) storage: Arc<dyn ObjectStoreService>,
}

impl PennylaneSync {
    pub(crate) fn new(
        store: Arc<Store>,
        client: Arc<PennylaneClient>,
        storage: Arc<dyn ObjectStoreService>,
    ) -> Self {
        let token_cache = Cache::builder()
            .expire_after(OauthAccessTokenExpiry)
            .max_capacity(500)
            .build();

        Self {
            store,
            client,
            token_cache,
            storage,
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

        Ok(succeeded_customers
            .into_iter()
            .chain(succeeded_invoices)
            .chain(succeeded_cus_outbox)
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
            .list_detailed_invoices_by_ids(ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let invoice_events = invoices
            .into_iter()
            .flat_map(|invoice| {
                let inv_id = invoice.invoice.id;
                invoice_reqs.iter().filter_map(move |(sync_inv, msg_id)| {
                    if sync_inv.id == inv_id {
                        Some((invoice.clone(), *msg_id))
                    } else {
                        None
                    }
                })
            })
            .collect_vec();

        let succeeded = self.sync_detailed_invoices(conn, invoice_events).await?;

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

            let res = match event.get_pennylane_id(conn.id) {
                Some(pennylane_id) => {
                    let company = Self::convert_to_update_company(*event);

                    self.client
                        .update_company_customer(pennylane_id, company, &conn.access_token)
                        .await
                }
                None => {
                    let company = Self::convert_to_new_company(*event);

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

    async fn sync_detailed_invoices(
        &self,
        conn: &PennylaneConnector,
        invoices: Vec<(DetailedInvoice, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if invoices.is_empty() {
            return Ok(Vec::new());
        }

        let mut succeeded_msgs = vec![];
        for (invoice, msg_id) in invoices {
            let res = self.sync_detailed_invoice(conn, invoice, msg_id).await;

            match res {
                Ok(succeeded_msg_id) => {
                    succeeded_msgs.push(succeeded_msg_id);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to sync detailed invoice with MessageId: {:?}, error: {:?}",
                        msg_id,
                        e
                    );
                }
            }
        }

        Ok(succeeded_msgs)
    }

    async fn mark_invoice_as_paid(
        &self,
        conn: &PennylaneConnector,
        pennylane_id: i64,
    ) -> PgmqResult<()> {
        let res = self
            .client
            .mark_customer_invoice_as_paid(pennylane_id, &conn.access_token)
            .await;

        if res.is_err() {
            log::warn!(
                "Failed to mark invoice {} as paid in pennylane: {:?}",
                pennylane_id,
                res
            );
        } else {
            log::info!("Invoice {} marked as paid in pennylane", pennylane_id);
        }

        Ok(())
    }

    async fn sync_detailed_invoice(
        &self,
        conn: &PennylaneConnector,
        invoice: DetailedInvoice,
        msg_id: MessageId,
    ) -> PgmqResult<MessageId> {
        let pennylane_inv_id = invoice
            .invoice
            .conn_meta
            .and_then(|x| x.get_pennylane_id(conn.id));

        if let Some(id) = pennylane_inv_id {
            log::info!(
                "Invoice {} was already synced to pennylane {}",
                invoice.invoice.id,
                id
            );

            if invoice.invoice.amount_due == 0 {
                self.mark_invoice_as_paid(conn, id).await?;
            }

            return Ok(msg_id);
        }

        let pennylane_cus_id = invoice
            .customer
            .conn_meta
            .and_then(|x| x.get_pennylane_id(conn.id));

        let pennylane_cus_id = match pennylane_cus_id {
            Some(id) => id,
            None => {
                log::warn!(
                    "Customer {} has no pennylane id, skipping invoice {}",
                    invoice.customer.id,
                    invoice.invoice.id
                );
                return Ok(msg_id);
            }
        };

        if let Some(pdf_id) = invoice
            .invoice
            .pdf_document_id
        {
            let currency = match rusty_money::iso::find(&invoice.invoice.currency) {
                Some(currency) => currency,
                None => {
                    log::warn!(
                        "Currency {} not found in rusty_money, skipping invoice {}",
                        invoice.invoice.currency,
                        invoice.invoice.id
                    );
                    return Ok(msg_id);
                }
            };

            let pdf_bytes = self
                .storage
                .retrieve(pdf_id, Prefix::InvoicePdf)
                .await
                .change_context(PgmqError::HandleMessages)?;

            let attachment = NewAttachment {
                filename: format!("{}.pdf", invoice.invoice.id),
                file: pdf_bytes,
                media_type: MediaType::ApplicationPdf,
            };

            let created = self
                .client
                .create_attachment(attachment, &conn.access_token)
                .await
                .change_context(PgmqError::HandleMessages)?;

            // let tax_amount = invoice.invoice.tax_amount.to_unit(currency.exponent as u8);
            // todo revisit me
            let tax_amount = Decimal::ZERO;
            let total_amount = invoice.invoice.total.to_unit(currency.exponent as u8);
            let total_before_tax = total_amount - tax_amount;
            //let tax_rate = (invoice.invoice.tax_rate as i64).to_unit(currency.exponent as u8);

            let to_sync = NewCustomerInvoiceImport {
                file_attachment_id: created.id,
                customer_id: pennylane_cus_id,
                external_reference: Some(invoice.invoice.id.as_proto()),
                invoice_number: Some(invoice.invoice.invoice_number),
                date: invoice.invoice.invoice_date,
                deadline: invoice
                    .invoice
                    .due_at
                    .as_ref()
                    .map(|x| x.date())
                    .unwrap_or(invoice.invoice.invoice_date),
                currency: invoice.invoice.currency,
                currency_amount_before_tax: total_before_tax.to_string(),
                currency_amount: total_amount.to_string(),
                currency_tax: tax_amount.to_string(),
                invoice_lines: invoice
                    .invoice
                    .line_items
                    .into_iter()
                    .map(|x| {
                        let total_amount = x.total.to_unit(currency.exponent as u8);
                        // todo revisit this field and have a dedicated field in the invoice line for tax amount
                        let tax_amount = 0;

                        CustomerInvoiceLine {
                            currency_amount: total_amount.to_string(),
                            currency_tax: tax_amount.to_string(),
                            label: x.name,
                            quantity: x.quantity.unwrap_or(Decimal::ONE),
                            raw_currency_unit_price: x
                                .unit_price
                                .unwrap_or(x.subtotal.to_unit(currency.exponent as u8))
                                .to_string(),
                            unit: "".to_string(),
                            vat_rate: "exempt".to_string(), // todo update me after we have tax implemented
                            description: None,
                        }
                    })
                    .collect_vec(),
            };

            let res = self
                .client
                .import_customer_invoice(to_sync, &conn.access_token)
                .await;

            match res {
                Ok(res) => {
                    self.store
                        .patch_invoice_conn_meta(
                            invoice.invoice.id,
                            conn.id,
                            ConnectorProviderEnum::Pennylane,
                            res.id.to_string().as_str(),
                        )
                        .await
                        .change_context(PgmqError::HandleMessages)?;

                    if invoice.invoice.amount_due == 0 {
                        self.mark_invoice_as_paid(conn, res.id).await?;
                    }

                    log::info!(
                        "Invoice {} synced to pennylane [id={}]",
                        invoice.invoice.id,
                        res.id
                    );
                }
                Err(e) => {
                    log::warn!(
                        "Failed to sync invoice {} to pennylane: {:?}",
                        invoice.invoice.id,
                        e
                    );

                    let status_code = e.status_code();

                    if status_code.is_some_and(|x| x < 500 && x != 429 && x != 409) {
                        return Ok(msg_id);
                    }

                    return Err(report!(PgmqError::HandleMessages).attach(e));
                }
            }
        }

        Ok(msg_id)
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

    fn convert_to_new_company(event: CustomerEvent) -> NewCompany {
        let billing_address = Self::convert_to_billing_address(event.billing_address.as_ref());

        NewCompany {
            name: event.name,
            billing_address,
            billing_email: event.billing_email,
            phone: event.phone.clone(),
            external_reference: event.customer_id.as_proto(),
            vat_number: event.vat_number,
            emails: event.invoicing_emails,
            billing_iban: None,
        }
    }

    fn convert_to_update_company(event: CustomerEvent) -> UpdateCompany {
        let billing_address = Self::convert_to_billing_address(event.billing_address.as_ref());

        UpdateCompany {
            name: event.name,
            billing_address,
            billing_email: event.billing_email.clone(),
            phone: event.phone.clone(),
            external_reference: event.customer_id.as_proto(),
            vat_number: event.vat_number.clone(),
            emails: event.invoicing_emails,
            billing_iban: None,
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
