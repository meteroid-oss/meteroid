//! Pushes customers, invoices and credit notes to the Pennylane company a tenant connected, and
//! reconciles invoices as they are paid.
//!
//! A document Pennylane cannot take as it stands is recorded as a sync issue the merchant sees and
//! can retry, and its message is acknowledged; only failures that may pass on their own are
//! retried by the queue.

mod mapping;

use crate::pgmq::PgmqResult;
use crate::pgmq::outbox::{native_or_outbox, pennylane_from_outbox};
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use chrono::NaiveDate;
use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId, InvoicingEntityId, TenantId};
use common_domain::pgmq::{MessageId, ReadCt};
use error_stack::Report;
use itertools::Itertools;
use mapping::Unsyncable;
use meteroid_app::services::storage::{self, ObjectStoreService};
use meteroid_store::Store;
use meteroid_store::domain::connector_sync_issues::SyncResource;
use meteroid_store::domain::connectors::ConnectorAccessToken;
use meteroid_store::domain::enums::{
    CreditNoteStatus, InvoicePaymentStatus, InvoiceStatusEnum, PaymentProvider, PaymentStatusEnum,
    PaymentTypeEnum,
};
use meteroid_store::domain::pgmq::{PennylaneSyncRequestEvent, PgmqMessage};
use meteroid_store::domain::{ConnectorProviderEnum, Customer, DetailedInvoice, Invoice};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::connector_sync_issues::ConnectorSyncIssuesInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::historical_rates::HistoricalRatesInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{CreditNoteInterface, CustomersInterface, InvoiceInterface};
use pennylane_client::client::PennylaneClient;
use pennylane_client::customer_invoices::{
    CustomerInvoiceImport, ImportedInvoiceUpdate, TransactionReference,
};
use pennylane_client::error::PennylaneError;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct PennylaneSync {
    store: Arc<Store>,
    client: Arc<PennylaneClient>,
    storage: Arc<dyn ObjectStoreService>,
    /// The read count past which the queue dead-letters a message.
    max_read_count: ReadCt,
}

impl PennylaneSync {
    pub(crate) fn new(
        store: Arc<Store>,
        client: Arc<PennylaneClient>,
        storage: Arc<dyn ObjectStoreService>,
        max_read_count: ReadCt,
    ) -> Self {
        Self {
            store,
            client,
            storage,
            max_read_count,
        }
    }

    /// Read afresh for every batch: the connector's scope, or the connector itself, may have
    /// changed since the last one.
    async fn connection(
        &self,
        tenant_id: TenantId,
    ) -> Result<Option<(ConnectorAccessToken, bool)>, String> {
        let error =
            |err: Report<StoreError>| format!("Could not load the Pennylane connector: {err:?}");
        let Some(connector) = self
            .store
            .get_pennylane_connector(tenant_id)
            .await
            .map_err(error)?
        else {
            return Ok(None);
        };
        let token = self
            .store
            .get_pennylane_connector_access_token(tenant_id)
            .await
            .map_err(error)?;
        Ok(token.map(|token| (token, connector.credentials_error().is_some())))
    }

    async fn sync_tenant(
        &self,
        tenant_id: TenantId,
        jobs: Jobs,
        read_counts: &HashMap<i64, ReadCt>,
    ) -> HandleResult {
        let mut result = HandleResult::from_succeeded(vec![]);

        let (token, flagged) = match self.connection(tenant_id).await {
            Ok(Some(connection)) => connection,
            Ok(None) => {
                log::info!("No Pennylane connector for tenant {tenant_id}, dropping its messages");
                result.succeeded = jobs.all_messages();
                return result;
            }
            Err(err) => {
                result.failed = jobs
                    .all_messages()
                    .into_iter()
                    .map(|id| (id, err.clone()))
                    .collect();
                return result;
            }
        };

        let mut sync = TenantSync {
            worker: self,
            tenant_id,
            token,
            flagged,
            token_reloaded: false,
            unauthorized: None,
            synced_customers: HashMap::new(),
        };

        let resources = jobs
            .customers
            .into_iter()
            .map(|(id, msgs)| (SyncResource::Customer(id), msgs))
            .chain(
                jobs.invoices
                    .into_iter()
                    .map(|(id, msgs)| (SyncResource::Invoice(id), msgs)),
            )
            .chain(
                jobs.credit_notes
                    .into_iter()
                    .map(|(id, msgs)| (SyncResource::CreditNote(id), msgs)),
            );

        for (resource, msgs) in resources {
            let outcome = sync.sync_resource(resource).await;
            let final_attempt = msgs.iter().any(|id| {
                read_counts
                    .get(&id.0)
                    .is_some_and(|ct| ct.0 > self.max_read_count.0)
            });
            sync.settle(resource, outcome, msgs, final_attempt, &mut result)
                .await;
        }

        result
    }
}

/// The resources of one tenant's messages, each synced once however many messages name it.
#[derive(Default)]
struct Jobs {
    customers: Vec<(CustomerId, Vec<MessageId>)>,
    invoices: Vec<(InvoiceId, Vec<MessageId>)>,
    credit_notes: Vec<(CreditNoteId, Vec<MessageId>)>,
}

impl Jobs {
    fn from_events(events: Vec<(MessageId, PennylaneSyncRequestEvent)>) -> Self {
        let mut customers = vec![];
        let mut invoices = vec![];
        let mut credit_notes = vec![];
        for (msg, event) in events {
            match event {
                PennylaneSyncRequestEvent::CustomerOutbox(e) => {
                    customers.push((e.customer_id, msg))
                }
                PennylaneSyncRequestEvent::Customer(e) => customers.push((e.id, msg)),
                PennylaneSyncRequestEvent::Invoice(e) => invoices.push((e.id, msg)),
                PennylaneSyncRequestEvent::CreditNote(e) => credit_notes.push((e.id, msg)),
            }
        }
        Jobs {
            customers: group(customers),
            invoices: group(invoices),
            credit_notes: group(credit_notes),
        }
    }

    fn all_messages(&self) -> Vec<MessageId> {
        [
            messages(&self.customers),
            messages(&self.invoices),
            messages(&self.credit_notes),
        ]
        .concat()
    }
}

fn messages<K>(jobs: &[(K, Vec<MessageId>)]) -> Vec<MessageId> {
    jobs.iter().flat_map(|(_, msgs)| msgs.clone()).collect()
}

fn group<K: Eq + Hash + Copy>(items: Vec<(K, MessageId)>) -> Vec<(K, Vec<MessageId>)> {
    let mut order = vec![];
    let mut grouped: HashMap<K, Vec<MessageId>> = HashMap::new();
    for (key, msg) in items {
        grouped
            .entry(key)
            .or_insert_with(|| {
                order.push(key);
                vec![]
            })
            .push(msg);
    }
    order
        .into_iter()
        .map(|key| (key, grouped.remove(&key).unwrap_or_default()))
        .collect()
}

enum Failure {
    /// May pass on its own: the message is retried.
    Transient(String),
    /// Sending it again changes nothing until the merchant acts.
    Unsyncable(String),
    /// Pennylane refused the token: nothing goes through until the company is reconnected.
    Unauthorized(String),
}

impl From<Unsyncable> for Failure {
    fn from(err: Unsyncable) -> Self {
        Failure::Unsyncable(err.0)
    }
}

impl From<Report<StoreError>> for Failure {
    fn from(err: Report<StoreError>) -> Self {
        log::warn!("Pennylane sync store error: {err:?}");
        Failure::Transient(format!(
            "Meteroid could not read or save it: {}",
            err.current_context()
        ))
    }
}

/// A create follows a lookup by external reference, so a conflict is not this document already
/// being there: it is another one, such as an invoice number used in Pennylane by hand. Only a
/// file still being processed settles on its own.
fn refused_create(what: &str) -> impl FnOnce(PennylaneError) -> Failure + '_ {
    move |err| {
        let message = err.message().to_lowercase();
        let pending_file = message.contains("file") || message.contains("attachment");
        if err.status() == Some(409) && !pending_file {
            Failure::Unsyncable(format!("Pennylane refused the {what}: {}", err.message()))
        } else {
            refused(what)(err)
        }
    }
}

fn refused(what: &str) -> impl FnOnce(PennylaneError) -> Failure + '_ {
    move |err| {
        if err.is_unauthorized() {
            Failure::Unauthorized(format!(
                "Pennylane refused the connection ({}). Reconnect Pennylane, then retry.",
                err.message()
            ))
        } else if err.is_retryable() {
            Failure::Transient(format!("Pennylane did not take the {what}: {err}"))
        } else {
            Failure::Unsyncable(format!("Pennylane refused the {what}: {}", err.message()))
        }
    }
}

struct TenantSync<'a> {
    worker: &'a PennylaneSync,
    tenant_id: TenantId,
    token: ConnectorAccessToken,
    /// The connector carries a credentials error, cleared by the first sync that goes through.
    flagged: bool,
    /// A refused token is reloaded once per batch: another process may have refreshed it.
    token_reloaded: bool,
    /// Set once Pennylane refused the token; the remaining resources are not sent.
    unauthorized: Option<String>,
    /// Pennylane ids of the customers synced in this batch, fresher than the documents loaded.
    synced_customers: HashMap<CustomerId, i64>,
}

impl TenantSync<'_> {
    fn store(&self) -> &Store {
        &self.worker.store
    }

    fn client(&self) -> &PennylaneClient {
        &self.worker.client
    }

    fn in_scope(&self, entity: InvoicingEntityId) -> bool {
        self.token
            .invoicing_entity_id
            .is_none_or(|scope| scope == entity)
    }

    async fn settle(
        &mut self,
        resource: SyncResource,
        outcome: Result<(), Failure>,
        msgs: Vec<MessageId>,
        final_attempt: bool,
        result: &mut HandleResult,
    ) {
        let store = self.worker.store.clone();
        let connector_id = self.token.connector_id;
        let tenant_id = self.tenant_id;

        let issue = match outcome {
            Ok(()) => {
                if let Err(err) = store
                    .clear_sync_issue(tenant_id, connector_id, resource)
                    .await
                {
                    log::warn!("Could not clear the Pennylane sync issue of {resource:?}: {err:?}");
                }
                result.succeeded.extend(msgs);
                return;
            }
            Err(Failure::Unsyncable(message)) => message,
            Err(Failure::Unauthorized(message)) => {
                if self.unauthorized.is_none() {
                    if let Err(err) = store
                        .flag_connector_credentials(tenant_id, connector_id, Some(message.clone()))
                        .await
                    {
                        log::warn!(
                            "Could not flag the Pennylane connector {connector_id}: {err:?}"
                        );
                    }
                    self.unauthorized = Some(message.clone());
                }
                message
            }
            Err(Failure::Transient(message)) if !final_attempt => {
                log::warn!("Pennylane sync of {resource:?} will be retried: {message}");
                result
                    .failed
                    .extend(msgs.into_iter().map(|id| (id, message.clone())));
                return;
            }
            Err(Failure::Transient(message)) => {
                format!("Still failing after several attempts: {message}")
            }
        };

        log::info!("Pennylane sync issue on {resource:?}: {issue}");
        match store
            .record_sync_issue(tenant_id, connector_id, resource, &issue)
            .await
        {
            Ok(()) => result.succeeded.extend(msgs),
            Err(err) => result
                .failed
                .extend(msgs.into_iter().map(|id| HandleResult::fail(id, &err))),
        }
    }

    async fn sync_resource(&mut self, resource: SyncResource) -> Result<(), Failure> {
        let Some(lock) = self.store().try_lock_sync_resource(resource).await? else {
            return Err(Failure::Transient(
                "Another worker is syncing it right now".to_owned(),
            ));
        };
        let mut outcome = self.dispatch(resource).await;
        if matches!(outcome, Err(Failure::Unauthorized(_))) && self.reload_token().await {
            outcome = self.dispatch(resource).await;
        }
        lock.release().await;

        if outcome.is_ok() && self.flagged {
            self.flagged = false;
            if let Err(err) = self
                .store()
                .flag_connector_credentials(self.tenant_id, self.token.connector_id, None)
                .await
            {
                log::warn!("Could not clear the Pennylane connector's credentials error: {err:?}");
            }
        }
        outcome
    }

    async fn dispatch(&mut self, resource: SyncResource) -> Result<(), Failure> {
        match resource {
            SyncResource::Customer(id) => self.sync_customer_by_id(id).await,
            SyncResource::Invoice(id) => self.sync_invoice(id).await,
            SyncResource::CreditNote(id) => self.sync_credit_note(id).await,
        }
    }

    /// Whether a different token was loaded.
    async fn reload_token(&mut self) -> bool {
        if self.token_reloaded {
            return false;
        }
        self.token_reloaded = true;
        match self
            .store()
            .get_pennylane_connector_access_token(self.tenant_id)
            .await
        {
            Ok(Some(token))
                if token.connector_id == self.token.connector_id
                    && token.access_token.expose_secret()
                        != self.token.access_token.expose_secret() =>
            {
                self.token = token;
                true
            }
            Ok(_) => false,
            Err(err) => {
                log::warn!("Could not reload the Pennylane token: {err:?}");
                false
            }
        }
    }

    fn check_authorized(&self) -> Result<(), Failure> {
        match &self.unauthorized {
            Some(message) => Err(Failure::Unauthorized(message.clone())),
            None => Ok(()),
        }
    }

    async fn sync_customer_by_id(&mut self, id: CustomerId) -> Result<(), Failure> {
        let customer = self
            .store()
            .list_customers_by_ids_global(vec![id])
            .await?
            .into_iter()
            .find(|customer| customer.tenant_id == self.tenant_id);
        match customer {
            Some(customer) if self.in_scope(customer.invoicing_entity_id) => {
                self.check_authorized()?;
                self.sync_customer(&customer).await.map(|_| ())
            }
            _ => Ok(()),
        }
    }

    /// Creates or updates the customer, adopting the one an earlier attempt may have created.
    async fn sync_customer(&mut self, customer: &Customer) -> Result<i64, Failure> {
        let upsert = mapping::customer_upsert(customer);
        let token = &self.token.access_token.clone();
        let known = customer
            .conn_meta
            .as_ref()
            .and_then(|meta| meta.get_pennylane_id(self.token.connector_id));

        let updated = match known {
            Some(id) => match self.client().update_customer(id, &upsert, token).await {
                Ok(updated) => Some(updated.id),
                Err(err) if err.is_not_found() => None,
                Err(err) => return Err(refused("customer")(err)),
            },
            None => None,
        };

        let id = match updated {
            Some(id) => id,
            None => match self
                .client()
                .find_customer(upsert.external_reference(), token)
                .await
                .map_err(refused("customer lookup"))?
            {
                Some(existing)
                    if existing
                        .customer_type
                        .is_some_and(|kind| kind != upsert.customer_type()) =>
                {
                    return Err(Failure::Unsyncable(format!(
                        "{} is a {:?} customer in Pennylane, which cannot become a {:?} one: \
                         change its type back, or remove it from Pennylane",
                        customer.name,
                        existing.customer_type.unwrap_or(upsert.customer_type()),
                        upsert.customer_type(),
                    )));
                }
                Some(existing) => {
                    self.client()
                        .update_customer(existing.id, &upsert, token)
                        .await
                        .map_err(refused("customer"))?
                        .id
                }
                None => {
                    self.client()
                        .create_customer(&upsert, token)
                        .await
                        .map_err(refused_create("customer"))?
                        .id
                }
            },
        };

        self.synced_customers.insert(customer.id, id);
        if known != Some(id) {
            self.store()
                .patch_customer_conn_meta(
                    customer.tenant_id,
                    customer.id,
                    self.token.connector_id,
                    ConnectorProviderEnum::Pennylane,
                    &id.to_string(),
                    &self.token.external_company_id,
                )
                .await?;
        }
        Ok(id)
    }

    async fn ensure_customer(&mut self, customer: &Customer) -> Result<i64, Failure> {
        let known = self
            .synced_customers
            .get(&customer.id)
            .copied()
            .or_else(|| {
                customer
                    .conn_meta
                    .as_ref()
                    .and_then(|meta| meta.get_pennylane_id(self.token.connector_id))
            });
        match known {
            Some(id) => Ok(id),
            None => self.sync_customer(customer).await,
        }
    }

    async fn load_invoice(&self, id: InvoiceId) -> Result<Option<DetailedInvoice>, Failure> {
        Ok(self
            .store()
            .list_detailed_invoices_by_ids(vec![id])
            .await?
            .into_iter()
            .find(|detailed| detailed.invoice.tenant_id == self.tenant_id))
    }

    async fn sync_invoice(&mut self, id: InvoiceId) -> Result<(), Failure> {
        let Some(detailed) = self.load_invoice(id).await? else {
            return Ok(());
        };
        let invoice = &detailed.invoice;
        if !self.in_scope(invoice.invoicing_entity_id) || !is_issued(invoice) {
            return Ok(());
        }
        self.check_authorized()?;

        let pennylane_id = self.ensure_invoice(&detailed).await?;
        self.reconcile_payment(invoice, pennylane_id).await
    }

    async fn ensure_invoice(&mut self, detailed: &DetailedInvoice) -> Result<i64, Failure> {
        let invoice = &detailed.invoice;
        if let Some(id) = invoice
            .conn_meta
            .as_ref()
            .and_then(|meta| meta.get_pennylane_id(self.token.connector_id))
        {
            return Ok(id);
        }

        let token = &self.token.access_token.clone();
        let imported = match self
            .client()
            .find_customer_invoice(&invoice.id.as_proto(), token)
            .await
            .map_err(refused("invoice lookup"))?
        {
            Some(existing) => existing.id,
            None => {
                let pdf_id = invoice.pdf_document_id.ok_or_else(|| {
                    Failure::Transient("The invoice PDF is not generated yet".to_owned())
                })?;
                let customer_id = self.ensure_customer(&detailed.customer).await?;
                let reference = match self.settlement_reference(invoice).await {
                    Ok(Some(reference)) => reference,
                    Ok(None) | Err(_) => TransactionReference::bank_label(&invoice.invoice_number),
                };
                let eur_rate = self.eur_rate(&invoice.currency, invoice.invoice_date).await;
                let pending =
                    mapping::invoice_import(invoice, customer_id, Some(reference), eur_rate)?;
                let attachment_id = self.upload_pdf(pdf_id, &invoice.invoice_number).await?;
                self.client()
                    .import_customer_invoice(&pending.with_attachment(attachment_id), token)
                    .await
                    .map_err(refused_create("invoice"))?
                    .id
            }
        };

        self.store()
            .patch_invoice_conn_meta(
                invoice.id,
                self.token.connector_id,
                ConnectorProviderEnum::Pennylane,
                &imported.to_string(),
                &self.token.external_company_id,
            )
            .await?;
        Ok(imported)
    }

    /// A paid invoice is matched by Pennylane to the provider payment it receives through its own
    /// Stripe or GoCardless feed; one paid any other way is marked paid.
    async fn reconcile_payment(
        &mut self,
        invoice: &Invoice,
        pennylane_id: i64,
    ) -> Result<(), Failure> {
        if invoice.payment_status != InvoicePaymentStatus::Paid {
            return Ok(());
        }
        let token = &self.token.access_token.clone();
        let remote = self
            .client()
            .get_customer_invoice(pennylane_id, token)
            .await
            .map_err(refused("invoice"))?;
        if remote.paid {
            return Ok(());
        }

        match self.settlement_reference(invoice).await? {
            Some(reference) => {
                let current = serde_json::to_value(&reference).ok();
                if remote.transaction_reference != current {
                    self.client()
                        .update_imported_customer_invoice(
                            pennylane_id,
                            &ImportedInvoiceUpdate {
                                transaction_reference: reference,
                            },
                            token,
                        )
                        .await
                        .map_err(refused("payment reference"))?;
                }
                Ok(())
            }
            None => self
                .client()
                .mark_customer_invoice_as_paid(pennylane_id, token)
                .await
                .map_err(refused("payment")),
        }
    }

    /// The Stripe or GoCardless payment that settled the whole invoice on its own, as Pennylane
    /// matches it; `None` when it was settled any other way, or by several payments or credit,
    /// which a single reference would leave partly open in Pennylane.
    async fn settlement_reference(
        &self,
        invoice: &Invoice,
    ) -> Result<Option<TransactionReference>, Failure> {
        if invoice.applied_credits != 0 {
            return Ok(None);
        }
        let settled: Vec<_> = self
            .store()
            .list_payment_tx_by_invoice_id(self.tenant_id, invoice.id)
            .await?
            .into_iter()
            .filter(|tx| {
                tx.transaction.status == PaymentStatusEnum::Settled
                    && tx.transaction.payment_type == PaymentTypeEnum::Payment
            })
            .collect();
        let [payment] = settled.as_slice() else {
            return Ok(None);
        };
        let (Some(provider_id), Some(method)) = (
            payment.transaction.provider_transaction_id.as_deref(),
            payment.method.as_ref(),
        ) else {
            return Ok(None);
        };
        if payment.transaction.amount != invoice.total {
            return Ok(None);
        }

        let store = self.store();
        let connection = store
            .get_connection_by_id(&self.tenant_id, &method.connection_id)
            .await?;
        let connector = store
            .get_connector_with_data(connection.connector_id, self.tenant_id)
            .await?;

        Ok(match connector.payment_provider() {
            Some(PaymentProvider::Stripe) if provider_id.starts_with("pi_") => {
                Some(TransactionReference::stripe_payment(provider_id))
            }
            Some(PaymentProvider::Stripe)
                if provider_id.starts_with("ch_") || provider_id.starts_with("py_") =>
            {
                Some(TransactionReference::stripe_charge(provider_id))
            }
            Some(PaymentProvider::Gocardless) => {
                Some(TransactionReference::gocardless_payment(provider_id))
            }
            _ => None,
        })
    }

    async fn sync_credit_note(&mut self, id: CreditNoteId) -> Result<(), Failure> {
        let Some(credit_note) = self
            .store()
            .list_credit_notes_by_ids(vec![id])
            .await?
            .into_iter()
            .find(|credit_note| credit_note.tenant_id == self.tenant_id)
        else {
            return Ok(());
        };
        if !self.in_scope(credit_note.invoicing_entity_id)
            || credit_note.status == CreditNoteStatus::Draft
        {
            return Ok(());
        }
        self.check_authorized()?;

        let connector_id = self.token.connector_id;
        if credit_note
            .conn_meta
            .as_ref()
            .and_then(|meta| meta.get_pennylane_id(connector_id))
            .is_some()
        {
            return Ok(());
        }

        let token = &self.token.access_token.clone();
        let imported = match self
            .client()
            .find_customer_invoice(&credit_note.id.as_proto(), token)
            .await
            .map_err(refused("credit note lookup"))?
        {
            Some(existing) => existing.id,
            None => {
                let pdf_id = credit_note.pdf_document_id.ok_or_else(|| {
                    Failure::Transient("The credit note PDF is not generated yet".to_owned())
                })?;
                let invoice = self
                    .load_invoice(credit_note.invoice_id)
                    .await?
                    .ok_or_else(|| {
                        Failure::Unsyncable(format!(
                            "The invoice {} of credit note {} is gone",
                            credit_note.invoice_number, credit_note.credit_note_number
                        ))
                    })?;
                if !is_issued(&invoice.invoice) {
                    return Err(Failure::Unsyncable(format!(
                        "The invoice {} of credit note {} was not issued",
                        credit_note.invoice_number, credit_note.credit_note_number
                    )));
                }
                let credited_id = self.ensure_invoice(&invoice).await?;
                self.clear_issue(SyncResource::Invoice(invoice.invoice.id))
                    .await;
                let customer_id = self.ensure_customer(&invoice.customer).await?;
                let date = credit_note
                    .finalized_at
                    .unwrap_or(credit_note.created_at)
                    .date();
                let eur_rate = self.eur_rate(&credit_note.currency, date).await;
                let pending =
                    mapping::credit_note_import(&credit_note, customer_id, credited_id, eur_rate)?;
                let attachment_id = self
                    .upload_pdf(pdf_id, &credit_note.credit_note_number)
                    .await?;
                let linked = pending.with_attachment(attachment_id);
                match self.client().import_customer_invoice(&linked, token).await {
                    Ok(imported) => imported.id,
                    // Pennylane only credits what is still to be paid: the credit note of a paid
                    // invoice stands on its own, which books the same entries.
                    Err(err)
                        if !err.is_retryable()
                            && !err.is_unauthorized()
                            && err.status() != Some(409) =>
                    {
                        log::info!(
                            "Pennylane refused to link credit note {} to its invoice ({}), \
                             importing it on its own",
                            credit_note.id,
                            err.message()
                        );
                        let unlinked = CustomerInvoiceImport {
                            credited_invoice_id: None,
                            ..linked
                        };
                        self.client()
                            .import_customer_invoice(&unlinked, token)
                            .await
                            .map_err(refused_create("credit note"))?
                            .id
                    }
                    Err(err) => return Err(refused_create("credit note")(err)),
                }
            }
        };

        self.store()
            .patch_credit_note_conn_meta(
                credit_note.id,
                connector_id,
                ConnectorProviderEnum::Pennylane,
                &imported.to_string(),
                &self.token.external_company_id,
            )
            .await?;
        Ok(())
    }

    /// The rate a document's PDF converts it at; without one Pennylane converts it itself.
    async fn eur_rate(&self, currency: &str, date: NaiveDate) -> Option<Decimal> {
        if currency == "EUR" {
            return None;
        }
        match self
            .store()
            .get_historical_rate(currency, "EUR", date)
            .await
        {
            Ok(rate) => rate.and_then(|rate| Decimal::from_f64(rate.rate)),
            Err(err) => {
                log::warn!("No {currency} to EUR rate for {date}: {err:?}");
                None
            }
        }
    }

    async fn clear_issue(&self, resource: SyncResource) {
        if let Err(err) = self
            .store()
            .clear_sync_issue(self.tenant_id, self.token.connector_id, resource)
            .await
        {
            log::warn!("Could not clear the Pennylane sync issue of {resource:?}: {err:?}");
        }
    }

    async fn upload_pdf(
        &self,
        pdf_id: common_domain::ids::StoredDocumentId,
        number: &str,
    ) -> Result<i64, Failure> {
        let bytes = storage::read(self.worker.storage.as_ref(), self.store(), pdf_id)
            .await
            .map_err(|err| Failure::Transient(format!("Could not read the PDF: {err:?}")))?;
        let filename: String = number
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.client()
            .upload_pdf(&format!("{filename}.pdf"), bytes, &self.token.access_token)
            .await
            .map(|attachment| attachment.id)
            .map_err(refused("PDF"))
    }
}

/// Numbered and standing on its own: a consolidated invoice's parts are billed by their parent,
/// and a closed one is an empty period that was never numbered.
fn is_issued(invoice: &Invoice) -> bool {
    matches!(
        invoice.status,
        InvoiceStatusEnum::Finalized | InvoiceStatusEnum::Void | InvoiceStatusEnum::Uncollectible
    ) && invoice.consolidated_into_invoice_id.is_none()
}

#[async_trait::async_trait]
impl PgmqHandler for PennylaneSync {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let read_counts: HashMap<i64, ReadCt> =
            msgs.iter().map(|msg| (msg.msg_id.0, msg.read_ct)).collect();
        let decoded = native_or_outbox(msgs, pennylane_from_outbox);

        let mut result = HandleResult {
            succeeded: vec![],
            failed: decoded.failed,
        };

        let by_tenant = decoded
            .events
            .into_iter()
            .into_group_map_by(|(_, event)| event.tenant_id());

        let tasks = by_tenant
            .into_iter()
            .map(|(tenant_id, events)| {
                let worker = self.clone();
                let read_counts = read_counts.clone();
                let jobs = Jobs::from_events(events);
                let msg_ids = jobs.all_messages();
                let task =
                    tokio::spawn(
                        async move { worker.sync_tenant(tenant_id, jobs, &read_counts).await },
                    );
                (tenant_id, msg_ids, task)
            })
            .collect_vec();

        for (tenant_id, msg_ids, task) in tasks {
            match task.await {
                Ok(tenant_result) => {
                    result.succeeded.extend(tenant_result.succeeded);
                    result.failed.extend(tenant_result.failed);
                }
                Err(err) => {
                    tracing::warn!(tenant_id = %tenant_id, "Pennylane sync task failed: {err:?}");
                    result
                        .failed
                        .extend(msg_ids.into_iter().map(|id| HandleResult::fail(id, &err)));
                }
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_naming_one_resource_sync_it_once_in_order() {
        let grouped = group(vec![
            ("b", MessageId(1)),
            ("a", MessageId(2)),
            ("b", MessageId(3)),
        ]);
        assert_eq!(
            grouped,
            vec![
                ("b", vec![MessageId(1), MessageId(3)]),
                ("a", vec![MessageId(2)]),
            ]
        );
    }
}
