use crate::pgmq::PgmqResult;
use crate::pgmq::error::PgmqError;
use crate::pgmq::outbox::{native_or_outbox, pennylane_from_outbox};
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::country::CountryCode;
use common_domain::ids::{StoredDocumentId, TenantId};
use common_domain::pgmq::MessageId;
use common_logging::unwrapper::UnwrapLogger;
use common_utils::decimals::ToUnit;
use error_stack::{Report, ResultExt};
use futures::TryFutureExt;
use itertools::Itertools;
use meteroid_app::services::storage::{self, ObjectStoreService};
use meteroid_store::Store;
use meteroid_store::domain::connectors::ConnectorAccessToken;
use meteroid_store::domain::outbox_event::CustomerEvent;
use meteroid_store::domain::pgmq::{
    PennylaneSyncCreditNote, PennylaneSyncCustomer, PennylaneSyncInvoice,
    PennylaneSyncRequestEvent, PgmqMessage,
};
use meteroid_store::domain::{
    Address, ConnectorProviderEnum, CreditNote, DetailedInvoice, LineItem,
};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::{CreditNoteInterface, CustomersInterface, InvoiceInterface};
use moka::Expiry;
use moka::future::Cache;
use pennylane_client::client::PennylaneClient;
use pennylane_client::customer_credit_notes::{
    CustomerCreditNotesApi, NewCustomerCreditNoteImport,
};
use pennylane_client::customer_invoices::{
    CustomerInvoiceLine, CustomerInvoiceLineImputationDates, CustomerInvoicesApi,
    NewCustomerInvoiceImport,
};
use pennylane_client::customers::{BillingAddress, CustomersApi, NewCompany, UpdateCompany};
use pennylane_client::file_attachments::{FileAttachmentsApi, MediaType, NewAttachment};
use pennylane_client::vat_rate::VatRate;
use rust_decimal::Decimal;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// todo extract out common and reuse in hubspot and pennylane

#[derive(Clone)]
pub(crate) struct PennylaneSync {
    pub(crate) store: Arc<Store>,
    pub(crate) client: Arc<PennylaneClient>,
    pub(crate) token_cache: Cache<TenantId, Option<ConnectorAccessToken>>,
    pub(crate) storage: Arc<dyn ObjectStoreService>,
}

impl PennylaneSync {
    pub(crate) fn new(
        store: Arc<Store>,
        client: Arc<PennylaneClient>,
        storage: Arc<dyn ObjectStoreService>,
    ) -> Self {
        let token_cache = Cache::builder()
            .expire_after(OptConnectorAccessTokenExpiry)
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
        // One task per tenant, even when its messages are not contiguous in the batch.
        let by_tenant = events
            .into_iter()
            .into_group_map_by(|(evt, _)| evt.tenant_id());

        let mut tasks = vec![];

        for (tenant_id, events) in by_tenant {
            let store = self.store.clone();
            let cache = self.token_cache.clone();

            tasks.push((
                tenant_id,
                events,
                tokio::spawn(async move {
                    cache
                        .try_get_with(
                            tenant_id,
                            store
                                .get_pennylane_connector_access_token(tenant_id)
                                .map_err(|x| x.change_context(PgmqError::HandleMessages)),
                        )
                        .await
                }),
            ));
        }

        let mut connected_tenants = vec![];

        // messages belonging to not connected tenants
        let mut orphan_msg_ids = vec![];

        for (tenant_id, events, task) in tasks {
            match task.await {
                Ok(Ok(Some(connector))) => {
                    connected_tenants.push(ConnectedTenant {
                        tenant_id,
                        conn_token: connector,
                        events,
                    });
                }
                Ok(Ok(None)) => {
                    orphan_msg_ids.extend(events.into_iter().map(|(_, msg_id)| msg_id));
                    log::info!("No pennylane connector found for tenant {tenant_id}");
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

    #[tracing::instrument(skip_all, fields(tenant_id = %conn.tenant_id))]
    async fn sync_connected_tenant(&self, conn: ConnectedTenant) -> PgmqResult<Vec<MessageId>> {
        let mut customers_to_sync = vec![];
        let mut invoices_to_sync = vec![];
        let mut credit_notes_to_sync = vec![];
        let mut customer_outbox_to_sync = vec![];

        for (evt, msg) in conn.events {
            match evt {
                PennylaneSyncRequestEvent::Customer(data) => {
                    customers_to_sync.push((data, msg));
                }
                PennylaneSyncRequestEvent::Invoice(data) => {
                    invoices_to_sync.push((data, msg));
                }
                PennylaneSyncRequestEvent::CreditNote(data) => {
                    credit_notes_to_sync.push((data, msg));
                }
                PennylaneSyncRequestEvent::CustomerOutbox(data) => {
                    customer_outbox_to_sync.push((data, msg));
                }
            }
        }

        let conn = conn.conn_token;

        let succeeded_customers = self
            .sync_customers(&conn, customers_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync customers: {e:?}"));
        let succeeded_invoices = self
            .sync_invoices(&conn, invoices_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync invoices: {e:?}"));
        let succeeded_credit_notes = self
            .sync_credit_notes(&conn, credit_notes_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync credit notes: {e:?}"));
        let succeeded_cus_outbox = self
            .sync_customer_outbox(&conn, customer_outbox_to_sync)
            .await
            .unwrap_to_default_warn(|e| format!("Failed to sync customer outbox events: {e:?}"));

        Ok(succeeded_customers
            .into_iter()
            .chain(succeeded_invoices)
            .chain(succeeded_credit_notes)
            .chain(succeeded_cus_outbox)
            .collect_vec())
    }

    async fn sync_customers(
        &self,
        conn: &ConnectorAccessToken,
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
        conn: &ConnectorAccessToken,
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
        conn: &ConnectorAccessToken,
        outboxes: Vec<(Box<CustomerEvent>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if outboxes.is_empty() {
            return Ok(Vec::new());
        }

        let mut succeeded_msgs = vec![];

        for (event, msg_id) in outboxes {
            let customer_id = event.customer_id;
            let tenant_id = event.tenant_id;

            let res = if let Some(pennylane_id) = event.get_pennylane_id(conn.connector_id) {
                let company = &Self::convert_to_update_company(*event);

                self.client
                    .update_company_customer(pennylane_id, company, &conn.access_token)
                    .await
            } else {
                let company = &Self::convert_to_new_company(*event);

                self.client
                    .upsert_company_customer(company, &conn.access_token)
                    .await
            };

            match res {
                Ok(res) => {
                    self.store
                        .patch_customer_conn_meta(
                            tenant_id,
                            customer_id,
                            conn.connector_id,
                            ConnectorProviderEnum::Pennylane,
                            res.id.to_string().as_str(),
                            conn.external_company_id.as_str(),
                        )
                        .await
                        .change_context(PgmqError::HandleMessages)?;

                    log::info!("Customer {customer_id} synced to pennylane [id={}]", res.id);

                    succeeded_msgs.push(msg_id);
                }
                Err(e) => {
                    log::warn!("Failed to create/update customer in pennylane: {e:?}");
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
        conn: &ConnectorAccessToken,
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
                        "Failed to sync detailed invoice with MessageId: {msg_id:?}, error: {e:?}"
                    );
                }
            }
        }

        Ok(succeeded_msgs)
    }

    async fn mark_invoice_as_paid(
        &self,
        conn: &ConnectorAccessToken,
        pennylane_id: i64,
    ) -> PgmqResult<()> {
        let res = self
            .client
            .mark_customer_invoice_as_paid(pennylane_id, &conn.access_token)
            .await;

        if res.is_err() {
            log::warn!("Failed to mark invoice {pennylane_id} as paid in pennylane: {res:?}");
        } else {
            log::info!("Invoice {pennylane_id} marked as paid in pennylane");
        }

        Ok(())
    }

    async fn sync_detailed_invoice(
        &self,
        conn: &ConnectorAccessToken,
        invoice: DetailedInvoice,
        msg_id: MessageId,
    ) -> PgmqResult<MessageId> {
        if let Some(id) = self.ensure_invoice_synced(conn, &invoice).await?
            && invoice.invoice.amount_due == 0
        {
            self.mark_invoice_as_paid(conn, id).await?;
        }

        Ok(msg_id)
    }

    /// Pennylane id of the invoice, importing it first when it is not synced yet.
    /// `None` when it can never be synced as is (customer not synced, unknown currency);
    /// `Err` when it is worth retrying.
    async fn ensure_invoice_synced(
        &self,
        conn: &ConnectorAccessToken,
        invoice: &DetailedInvoice,
    ) -> PgmqResult<Option<i64>> {
        let pennylane_inv_id = invoice
            .invoice
            .conn_meta
            .as_ref()
            .and_then(|x| x.get_pennylane_id(conn.connector_id));

        if let Some(id) = pennylane_inv_id {
            log::info!(
                "Invoice {} was already synced to pennylane {}",
                invoice.invoice.id,
                id
            );

            return Ok(Some(id));
        }

        let pennylane_cus_id = invoice
            .customer
            .conn_meta
            .as_ref()
            .and_then(|x| x.get_pennylane_id(conn.connector_id));

        let Some(pennylane_cus_id) = pennylane_cus_id else {
            log::warn!(
                "Customer {} has no pennylane id, skipping invoice {}",
                invoice.customer.id,
                invoice.invoice.id
            );
            return Ok(None);
        };

        let Some(pdf_id) = invoice.invoice.pdf_document_id else {
            // Sync is triggered by InvoiceAccountingPdfGenerated, so a missing
            // PDF here is a transactional race — return Err to retry rather
            // than silently acking and losing the sync.
            log::warn!(
                "Invoice {} has no PDF document yet, will retry",
                invoice.invoice.id
            );
            return Err(Report::new(PgmqError::HandleMessages));
        };

        let Some(currency) = meteroid_money::iso::find(&invoice.invoice.currency) else {
            log::warn!(
                "Currency {} not found in meteroid_money, skipping invoice {}",
                invoice.invoice.currency,
                invoice.invoice.id
            );
            return Ok(None);
        };

        let exponent = currency.exponent as u8;

        let attachment_id = self
            .upload_pdf(conn, pdf_id, format!("{}.pdf", invoice.invoice.id))
            .await?;

        let tax_amount = invoice.invoice.tax_amount.to_unit(exponent);
        let total_amount = invoice.invoice.total.to_unit(exponent);
        let total_before_tax = total_amount - tax_amount;

        let billing_country = invoice
            .customer
            .billing_address
            .as_ref()
            .and_then(|x| x.country.as_ref());

        let to_sync = NewCustomerInvoiceImport {
            file_attachment_id: attachment_id,
            customer_id: pennylane_cus_id,
            external_reference: invoice.invoice.id.as_proto(),
            invoice_number: Some(invoice.invoice.invoice_number.clone()),
            date: invoice.invoice.invoice_date,
            deadline: invoice
                .invoice
                .due_at
                .as_ref()
                .map_or(invoice.invoice.invoice_date, chrono::NaiveDateTime::date),
            currency: invoice.invoice.currency.clone(),
            currency_amount_before_tax: total_before_tax.to_string(),
            currency_amount: total_amount.to_string(),
            currency_tax: tax_amount.to_string(),
            invoice_lines: Self::convert_to_invoice_lines(
                &invoice.invoice.line_items,
                exponent,
                billing_country,
            ),
        };

        let res = self
            .client
            .import_or_get_customer_invoice(&to_sync, &conn.access_token)
            .await;

        match res {
            Ok(res) => {
                self.store
                    .patch_invoice_conn_meta(
                        invoice.invoice.id,
                        conn.connector_id,
                        ConnectorProviderEnum::Pennylane,
                        res.id.to_string().as_str(),
                        conn.external_company_id.as_str(),
                    )
                    .await
                    .change_context(PgmqError::HandleMessages)?;

                log::info!(
                    "Invoice {} synced to pennylane [id={}]",
                    invoice.invoice.id,
                    res.id
                );

                Ok(Some(res.id))
            }
            Err(e) => {
                log::warn!(
                    "Failed to sync invoice {} to pennylane: {:?}",
                    invoice.invoice.id,
                    e
                );

                let status_code = e.status_code();

                if status_code.is_some_and(|x| x < 500 && x != 429 && x != 409) {
                    return Ok(None);
                }

                Err(Report::new(PgmqError::HandleMessages).attach(e))
            }
        }
    }

    async fn sync_credit_notes(
        &self,
        conn: &ConnectorAccessToken,
        credit_note_reqs: Vec<(Box<PennylaneSyncCreditNote>, MessageId)>,
    ) -> PgmqResult<Vec<MessageId>> {
        if credit_note_reqs.is_empty() {
            return Ok(Vec::new());
        }

        let ids = credit_note_reqs
            .iter()
            .map(|(cn, _)| cn.id)
            .unique()
            .collect_vec();

        let credit_notes = self
            .store
            .list_credit_notes_by_ids(ids)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let credit_note_events = credit_notes
            .into_iter()
            .flat_map(|credit_note| {
                let cn_id = credit_note.id;
                credit_note_reqs.iter().filter_map(move |(req, msg_id)| {
                    if req.id == cn_id {
                        Some((credit_note.clone(), *msg_id))
                    } else {
                        None
                    }
                })
            })
            .collect_vec();

        let mut succeeded_msgs = vec![];

        for (credit_note, msg_id) in credit_note_events {
            match self.sync_credit_note(conn, credit_note).await {
                Ok(()) => succeeded_msgs.push(msg_id),
                Err(e) => {
                    log::warn!(
                        "Failed to sync credit note with MessageId: {msg_id:?}, error: {e:?}"
                    );
                }
            }
        }

        Ok(succeeded_msgs)
    }

    /// An avoir is imported as a customer invoice with negative amounts pointing at the
    /// credited invoice, so the invoice has to reach Pennylane first.
    async fn sync_credit_note(
        &self,
        conn: &ConnectorAccessToken,
        credit_note: CreditNote,
    ) -> PgmqResult<()> {
        let already_synced = credit_note
            .conn_meta
            .as_ref()
            .and_then(|x| x.get_pennylane_id(conn.connector_id));

        if let Some(id) = already_synced {
            log::info!(
                "Credit note {} was already synced to pennylane {}",
                credit_note.id,
                id
            );
            return Ok(());
        }

        let Some(pdf_id) = credit_note.pdf_document_id else {
            log::warn!(
                "Credit note {} has no PDF document yet, will retry",
                credit_note.id
            );
            return Err(Report::new(PgmqError::HandleMessages));
        };

        let Some(currency) = meteroid_money::iso::find(&credit_note.currency) else {
            log::warn!(
                "Currency {} not found in meteroid_money, skipping credit note {}",
                credit_note.currency,
                credit_note.id
            );
            return Ok(());
        };

        let invoice = self
            .store
            .list_detailed_invoices_by_ids(vec![credit_note.invoice_id])
            .await
            .change_context(PgmqError::HandleMessages)?
            .into_iter()
            .next();

        let Some(invoice) = invoice else {
            log::warn!(
                "Invoice {} of credit note {} not found, skipping",
                credit_note.invoice_id,
                credit_note.id
            );
            return Ok(());
        };

        let Some(credited_invoice_id) = self.ensure_invoice_synced(conn, &invoice).await? else {
            log::warn!(
                "Invoice {} is not syncable to pennylane, skipping credit note {}",
                credit_note.invoice_id,
                credit_note.id
            );
            return Ok(());
        };

        let Some(pennylane_cus_id) = invoice
            .customer
            .conn_meta
            .as_ref()
            .and_then(|x| x.get_pennylane_id(conn.connector_id))
        else {
            log::warn!(
                "Customer {} has no pennylane id, skipping credit note {}",
                invoice.customer.id,
                credit_note.id
            );
            return Ok(());
        };

        let exponent = currency.exponent as u8;

        let attachment_id = self
            .upload_pdf(conn, pdf_id, format!("{}.pdf", credit_note.id))
            .await?;

        // Totals and line items are already negated at creation, which is what makes the
        // imported document an avoir on Pennylane's side.
        let tax_amount = credit_note.tax_amount.to_unit(exponent);
        let total_amount = credit_note.total.to_unit(exponent);
        let total_before_tax = total_amount - tax_amount;

        let date = credit_note
            .finalized_at
            .unwrap_or(credit_note.created_at)
            .date();

        let billing_country = invoice
            .customer
            .billing_address
            .as_ref()
            .and_then(|x| x.country.as_ref());

        let to_sync = NewCustomerCreditNoteImport {
            file_attachment_id: attachment_id,
            customer_id: pennylane_cus_id,
            external_reference: credit_note.id.as_proto(),
            invoice_number: Some(credit_note.credit_note_number),
            date,
            deadline: date,
            currency: credit_note.currency,
            currency_amount_before_tax: total_before_tax.to_string(),
            currency_amount: total_amount.to_string(),
            currency_tax: tax_amount.to_string(),
            credited_invoice_id,
            invoice_lines: Self::convert_to_invoice_lines(
                &credit_note.line_items,
                exponent,
                billing_country,
            ),
        };

        let res = self
            .client
            .import_or_get_customer_credit_note(&to_sync, &conn.access_token)
            .await;

        match res {
            Ok(res) => {
                self.store
                    .patch_credit_note_conn_meta(
                        credit_note.id,
                        conn.connector_id,
                        ConnectorProviderEnum::Pennylane,
                        res.id.to_string().as_str(),
                        conn.external_company_id.as_str(),
                    )
                    .await
                    .change_context(PgmqError::HandleMessages)?;

                log::info!(
                    "Credit note {} synced to pennylane [id={}]",
                    credit_note.id,
                    res.id
                );

                Ok(())
            }
            Err(e) => {
                log::warn!(
                    "Failed to sync credit note {} to pennylane: {:?}",
                    credit_note.id,
                    e
                );

                let status_code = e.status_code();

                if status_code.is_some_and(|x| x < 500 && x != 429 && x != 409) {
                    return Ok(());
                }

                Err(Report::new(PgmqError::HandleMessages).attach(e))
            }
        }
    }

    async fn upload_pdf(
        &self,
        conn: &ConnectorAccessToken,
        pdf_id: StoredDocumentId,
        filename: String,
    ) -> PgmqResult<i64> {
        let pdf_bytes = storage::read(self.storage.as_ref(), &self.store, pdf_id)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let attachment = NewAttachment {
            filename,
            file: pdf_bytes,
            media_type: MediaType::ApplicationPdf,
        };

        self.client
            .create_attachment(attachment, &conn.access_token)
            .await
            .change_context(PgmqError::HandleMessages)
            .map(|created| created.id)
    }

    fn convert_to_invoice_lines(
        line_items: &[LineItem],
        exponent: u8,
        billing_country: Option<&CountryCode>,
    ) -> Vec<CustomerInvoiceLine> {
        line_items
            .iter()
            .map(|x| {
                let total_amount = x.amount_total.to_unit(exponent);
                let tax_amount = x.tax_amount.to_unit(exponent);

                let vat_rate = if let Some(country) = billing_country {
                    VatRate::from_decimal(x.tax_rate, &country.code).unwrap_or(VatRate::EXEMPT)
                } else {
                    VatRate::EXEMPT
                };

                // For lines without an explicit unit_price (usage/tiered/etc.),
                // collapse to a single unit at the full subtotal price so
                // Pennylane's quantity * unit_price matches the line subtotal.
                // Credit note lines carry the sign on the amounts only, so mirror it
                // onto the unit price to keep that product reconciling.
                let (quantity, raw_unit_price) = match x.unit_price {
                    Some(p) if x.amount_subtotal < 0 => (x.quantity.unwrap_or(Decimal::ONE), -p),
                    Some(p) => (x.quantity.unwrap_or(Decimal::ONE), p),
                    None => (Decimal::ONE, x.amount_subtotal.to_unit(exponent)),
                };

                CustomerInvoiceLine {
                    currency_amount: total_amount.to_string(),
                    currency_tax: tax_amount.to_string(),
                    label: x.name.clone(),
                    quantity,
                    raw_currency_unit_price: raw_unit_price.to_string(),
                    unit: String::new(),
                    vat_rate,
                    description: None,
                    imputation_dates: Some(CustomerInvoiceLineImputationDates {
                        start_date: x.start_date,
                        end_date: x.end_date,
                    }),
                }
            })
            .collect_vec()
    }

    fn convert_to_billing_address(ba: Option<&Address>) -> BillingAddress {
        let address = ba.and_then(|x| x.line1.clone()).unwrap_or_default();

        let postal_code = ba.and_then(|x| x.zip_code.clone()).unwrap_or_default();
        let city = ba.and_then(|x| x.city.clone()).unwrap_or_default();

        let country_alpha2 = ba
            .and_then(|x| x.country.clone())
            .map(|c| c.code)
            .unwrap_or_default();

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
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let decoded = native_or_outbox(msgs, pennylane_from_outbox);
        let events = decoded
            .events
            .into_iter()
            .map(|(id, evt)| (evt, id))
            .collect();
        let (connected_tenants, orphan_msg_ids) = self.get_connected_tenants(events).await?;

        // messages that are not connected to any tenant should be marked as success
        let mut result = HandleResult {
            succeeded: orphan_msg_ids,
            failed: decoded.failed,
        };

        let tasks = connected_tenants
            .into_iter()
            .map(|connected| {
                let tenant_id = connected.tenant_id;
                let msg_ids = connected.events.iter().map(|(_, id)| *id).collect_vec();
                let task = tokio::spawn({
                    let value = self.clone();
                    async move { value.sync_connected_tenant(connected).await }
                });
                (tenant_id, msg_ids, task)
            })
            .collect_vec();

        for (tenant_id, msg_ids, task) in tasks {
            match task.await {
                Ok(Ok(ids)) => {
                    result.succeeded.extend(ids);
                }
                Ok(Err(e)) => {
                    tracing::warn!(tenant_id = %tenant_id, "Failed to sync connected tenant: {e:?}");
                    result
                        .failed
                        .extend(msg_ids.into_iter().map(|id| HandleResult::fail(id, &e)));
                }
                Err(e) => {
                    tracing::warn!(tenant_id = %tenant_id, "Sync task failed: {e:?}");
                    result
                        .failed
                        .extend(msg_ids.into_iter().map(|id| HandleResult::fail(id, &e)));
                }
            }
        }

        Ok(result)
    }
}

#[allow(dead_code)]
struct ConnectedTenant {
    tenant_id: TenantId,
    conn_token: ConnectorAccessToken,
    events: Vec<(PennylaneSyncRequestEvent, MessageId)>,
}

struct OptConnectorAccessTokenExpiry;

impl Expiry<TenantId, Option<ConnectorAccessToken>> for OptConnectorAccessTokenExpiry {
    fn expire_after_create(
        &self,
        _key: &TenantId,
        value: &Option<ConnectorAccessToken>,
        _created_at: Instant,
    ) -> Option<Duration> {
        if let Some(token) = value {
            let expires_at = token
                .expires_at
                .unwrap_or(chrono::Utc::now() + chrono::Duration::seconds(86400));

            expires_at
                .signed_duration_since(chrono::Utc::now())
                .to_std()
                .ok()
                .and_then(|x| x.checked_sub(Duration::from_secs(60))) // expire 1 minute earlier
                .or(Some(Duration::ZERO))
        } else {
            Some(Duration::from_secs(60))
        }
    }
}
