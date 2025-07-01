use crate::domain::enums::InvoiceType;
use crate::errors::StoreError;
use crate::store::Store;
use crate::{StoreResult, domain};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::PgConn;
use diesel_models::enums::{MrrMovementType, SubscriptionEventType};
use error_stack::{Report, bail};

use crate::domain::outbox_event::{InvoicePdfGeneratedEvent, OutboxEvent};
use crate::domain::pgmq::{
    PennylaneSyncInvoice, PennylaneSyncRequestEvent, PgmqMessageNew, PgmqQueue,
};
use crate::domain::{
    ConnectorProviderEnum, CursorPaginatedVec, CursorPaginationRequest, DetailedInvoice, Invoice,
    InvoiceNew, InvoiceWithCustomer, OrderByRequest, PaginatedVec, PaginationRequest,
};
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::customer_balance::CustomerBalance;
use crate::repositories::pgmq::PgmqInterface;
use common_domain::ids::{
    BaseId, ConnectorId, CustomerId, EventId, InvoiceId, StoredDocumentId, SubscriptionId, TenantId,
};
use diesel_models::customer_balance_txs::CustomerBalancePendingTxRow;
use diesel_models::invoices::{InvoiceRow, InvoiceRowNew};
use diesel_models::subscriptions::SubscriptionRow;
use tracing_log::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait InvoiceInterface {
    async fn get_detailed_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice>;

    async fn get_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Invoice>;

    #[allow(clippy::too_many_arguments)]
    async fn list_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        subscription_id: Option<SubscriptionId>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>>;

    async fn insert_invoice(&self, invoice: InvoiceNew) -> StoreResult<Invoice>;

    async fn insert_invoice_batch(&self, invoice: Vec<InvoiceNew>) -> StoreResult<Vec<Invoice>>;

    async fn list_invoices_to_finalize(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn list_outdated_invoices(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn list_invoices_by_ids(&self, ids: Vec<InvoiceId>) -> StoreResult<Vec<Invoice>>;

    async fn list_detailed_invoices_by_ids(
        &self,
        ids: Vec<InvoiceId>,
    ) -> StoreResult<Vec<DetailedInvoice>>;

    async fn save_invoice_documents(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        customer_id: CustomerId,

        pdf_id: StoredDocumentId,
        xml_id: Option<StoredDocumentId>,
    ) -> StoreResult<()>;

    async fn sync_invoices_to_pennylane(
        &self,
        ids: Vec<InvoiceId>,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn patch_invoice_conn_meta(
        &self,
        invoice_id: InvoiceId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl InvoiceInterface for Store {
    async fn get_detailed_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::find_detailed_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())
    }

    async fn get_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Invoice> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())
    }

    async fn list_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        subscription_id: Option<SubscriptionId>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>> {
        let mut conn = self.get_conn().await?;

        let rows = InvoiceRow::list(
            &mut conn,
            tenant_id,
            customer_id,
            subscription_id,
            status.map(Into::into),
            query,
            order_by.into(),
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<InvoiceWithCustomer> = PaginatedVec {
            items: rows
                .items
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn insert_invoice(&self, invoice: InvoiceNew) -> StoreResult<Invoice> {
        self.transaction(|conn| {
            async move { insert_invoice_tx(self, conn, invoice).await }.scope_boxed()
        })
        .await
    }

    async fn insert_invoice_batch(&self, invoice: Vec<InvoiceNew>) -> StoreResult<Vec<Invoice>> {
        self.transaction(|conn| {
            async move { insert_invoice_batch_tx(self, conn, invoice).await }.scope_boxed()
        })
        .await
    }

    async fn list_invoices_to_finalize(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_to_finalize(&mut conn, pagination.into())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let res: CursorPaginatedVec<Invoice> = CursorPaginatedVec {
            items: invoices
                .items
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            next_cursor: invoices.next_cursor,
        };

        Ok(res)
    }

    async fn list_outdated_invoices(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_outdated(&mut conn, pagination.into())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let res: CursorPaginatedVec<Invoice> = CursorPaginatedVec {
            items: invoices
                .items
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            next_cursor: invoices.next_cursor,
        };

        Ok(res)
    }

    async fn list_invoices_by_ids(&self, ids: Vec<InvoiceId>) -> StoreResult<Vec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        invoices
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_detailed_invoices_by_ids(
        &self,
        ids: Vec<InvoiceId>,
    ) -> StoreResult<Vec<DetailedInvoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_detailed_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        invoices
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()
    }

    async fn save_invoice_documents(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        customer_id: CustomerId,
        pdf_id: StoredDocumentId,
        xml_id: Option<StoredDocumentId>,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                InvoiceRow::save_invoice_documents(conn, id, tenant_id, pdf_id, xml_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let evt: PgmqMessageNew =
                    OutboxEvent::invoice_pdf_generated(InvoicePdfGeneratedEvent {
                        id: EventId::new(),
                        invoice_id: id,
                        tenant_id,
                        customer_id,
                        pdf_id,
                    })
                    .try_into()?;
                self.pgmq_send_batch_tx(conn, PgmqQueue::OutboxEvent, vec![evt])
                    .await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await?;

        Ok(())
    }

    async fn sync_invoices_to_pennylane(
        &self,
        ids: Vec<InvoiceId>,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let connector = self.get_pennylane_connector(tenant_id).await?;

        if connector.is_none() {
            bail!(StoreError::InvalidArgument(
                "No Pennylane connector found".to_string()
            ));
        }

        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.pgmq_send_batch(
            PgmqQueue::PennylaneSync,
            invoices
                .into_iter()
                .map(|invoice| {
                    PennylaneSyncRequestEvent::Invoice(Box::new(PennylaneSyncInvoice {
                        id: invoice.id,
                        tenant_id,
                    }))
                    .try_into()
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }

    async fn patch_invoice_conn_meta(
        &self,
        invoice_id: InvoiceId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::upsert_conn_meta(
            &mut conn,
            provider.into(),
            invoice_id,
            connector_id,
            external_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }
}

/*

TODO special cases :
- cancellation/all invoice => all mrr logs after that should be cancelled, unless reactivation
- cancellation : recalculate the mrr delta (as the one in the event was calculated before the invoice was created)
- consolidation if multiple events in the same day. Ex: new business + expansion = new business, or cancellation + reactivation => nothing
 */
async fn process_mrr(inserted: &Invoice, conn: &mut PgConn) -> StoreResult<()> {
    log::info!("Processing MRR logs for invoice {}", inserted.id);
    if inserted.invoice_type == InvoiceType::Recurring
        || inserted.invoice_type == InvoiceType::Adjustment
    {
        let subscription_id = inserted
            .subscription_id
            .ok_or(StoreError::ValueNotFound("subscription_id is null".into()))?;

        let subscription_events = diesel_models::subscription_events::SubscriptionEventRow::fetch_by_subscription_id_and_date(
            conn,
            subscription_id,
            inserted.invoice_date,
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let mut mrr_logs = vec![];

        for event in subscription_events {
            let mrr_delta = match event.mrr_delta {
                None | Some(0) => continue,
                Some(c) => c,
            };

            let movement_type = match event.event_type {
                // TODO
                // SubscriptionEventType::Created => continue,
                SubscriptionEventType::Created => MrrMovementType::NewBusiness, // TODO
                SubscriptionEventType::Activated => MrrMovementType::NewBusiness,
                SubscriptionEventType::Switch => {
                    if mrr_delta > 0 {
                        MrrMovementType::Expansion
                    } else {
                        MrrMovementType::Contraction
                    }
                }
                SubscriptionEventType::Cancelled => MrrMovementType::Churn,
                SubscriptionEventType::Reactivated => MrrMovementType::Reactivation,
                SubscriptionEventType::Updated => {
                    if mrr_delta > 0 {
                        MrrMovementType::Expansion
                    } else {
                        MrrMovementType::Contraction
                    }
                }
            };

            // TODO proper description from event_type + details
            let description = match event.event_type {
                SubscriptionEventType::Created => "Subscription created",
                SubscriptionEventType::Activated => "Subscription activated",
                SubscriptionEventType::Switch => "Switched plan",
                SubscriptionEventType::Cancelled => "Subscription cancelled",
                SubscriptionEventType::Reactivated => "Subscription reactivated",
                SubscriptionEventType::Updated => "Subscription updated",
            };

            if let Some(plan_version_id) = inserted.plan_version_id {
                let new_log = diesel_models::bi::BiMrrMovementLogRowNew {
                    id: Uuid::now_v7(),
                    description: description.to_string(),
                    movement_type,
                    net_mrr_change: mrr_delta,
                    currency: inserted.currency.clone(),
                    applies_to: inserted.invoice_date,
                    invoice_id: inserted.id,
                    credit_note_id: None,
                    plan_version_id,
                    tenant_id: inserted.tenant_id,
                };

                mrr_logs.push(new_log);
            }
        }

        let mrr_delta_cents: i64 = mrr_logs.iter().map(|l| l.net_mrr_change).sum();

        diesel_models::bi::BiMrrMovementLogRow::insert_movement_log_batch(conn, mrr_logs)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, mrr_delta_cents)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok(())
}

pub async fn insert_invoice_tx(
    store: &Store,
    tx: &mut PgConn,
    invoice: InvoiceNew,
) -> StoreResult<Invoice> {
    insert_invoice_batch_tx(store, tx, vec![invoice])
        .await?
        .pop()
        .ok_or(StoreError::InsertError.into())
}

async fn insert_invoice_batch_tx(
    store: &Store,
    tx: &mut PgConn,
    invoice: Vec<InvoiceNew>,
) -> StoreResult<Vec<Invoice>> {
    let insertable_invoice: Vec<InvoiceRowNew> = invoice
        .into_iter()
        .map(|c| c.try_into())
        .collect::<Result<_, _>>()?;

    let inserted: Vec<Invoice> = InvoiceRow::insert_invoice_batch(tx, insertable_invoice)
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .and_then(|v| {
            v.into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, Report<StoreError>>>()
        })?;

    // TODO batch
    for inv in inserted.iter() {
        process_mrr(inv, tx).await?; // TODO
        let final_invoice: Invoice = InvoiceRow::find_by_id(tx, inv.tenant_id, inv.id)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())?;

        store
            .internal
            .insert_outbox_events_tx(
                tx,
                vec![OutboxEvent::invoice_created((&final_invoice).into())],
            )
            .await?;
    }

    Ok(inserted)
}

// TODO unused (was in update_invoice_external_status)
async fn _process_pending_tx(conn: &mut PgConn, invoice_id: InvoiceId) -> StoreResult<()> {
    let pending_tx = CustomerBalancePendingTxRow::find_unprocessed_by_invoice_id(conn, invoice_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if let Some(pending_tx) = pending_tx {
        let tx_id = CustomerBalance::update(
            conn,
            pending_tx.customer_id,
            pending_tx.tenant_id,
            pending_tx.amount_cents,
            Some(invoice_id),
        )
        .await?
        .tx_id;

        CustomerBalancePendingTxRow::update_tx_id(conn, pending_tx.id, tx_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    Ok(())
}
