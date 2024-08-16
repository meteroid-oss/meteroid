use crate::domain::enums::{InvoiceExternalStatusEnum, InvoiceType};
use crate::errors::StoreError;
use crate::store::Store;
use crate::{domain, StoreResult};
use chrono::NaiveDateTime;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{MrrMovementType, SubscriptionEventType};
use diesel_models::PgConn;
use error_stack::Report;

use crate::compute::InvoiceLineInterface;
use crate::domain::{
    CursorPaginatedVec, CursorPaginationRequest, DetailedInvoice, Invoice, InvoiceLinesPatch,
    InvoiceNew, InvoiceWithCustomer, OrderByRequest, PaginatedVec, PaginationRequest,
};
use crate::repositories::customer_balance::CustomerBalance;
use crate::repositories::SubscriptionInterface;
use common_eventbus::Event;
use diesel_models::customer_balance_txs::CustomerBalancePendingTxRow;
use diesel_models::invoices::{InvoiceRow, InvoiceRowLinesPatch, InvoiceRowNew};
use diesel_models::subscriptions::SubscriptionRow;
use tracing_log::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait InvoiceInterface {
    async fn find_invoice_by_id(
        &self,
        tenant_id: Uuid,
        invoice_id: Uuid,
    ) -> StoreResult<DetailedInvoice>;

    async fn list_invoices(
        &self,
        tenant_id: Uuid,
        customer_id: Option<Uuid>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>>;

    async fn insert_invoice(&self, invoice: InvoiceNew) -> StoreResult<Invoice>;

    async fn insert_invoice_batch(&self, invoice: Vec<InvoiceNew>) -> StoreResult<Vec<Invoice>>;

    async fn update_invoice_external_status(
        &self,
        invoice_id: Uuid,
        tenant_id: Uuid,
        external_status: InvoiceExternalStatusEnum,
    ) -> StoreResult<()>;

    async fn list_invoices_to_finalize(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn finalize_invoice(&self, id: Uuid, tenant_id: Uuid) -> StoreResult<()>;

    async fn list_outdated_invoices(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn list_invoices_to_issue(
        &self,
        max_attempts: i32,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn invoice_issue_success(&self, id: Uuid, tenant_id: Uuid) -> StoreResult<()>;

    async fn invoice_issue_error(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        last_issue_error: &str,
    ) -> StoreResult<()>;

    async fn update_pending_finalization_invoices(&self, now: NaiveDateTime) -> StoreResult<()>;

    async fn refresh_invoice_data(&self, id: Uuid, tenant_id: Uuid)
        -> StoreResult<DetailedInvoice>;
}

#[async_trait::async_trait]
impl InvoiceInterface for Store {
    async fn find_invoice_by_id(
        &self,
        tenant_id: Uuid,
        invoice_id: Uuid,
    ) -> StoreResult<DetailedInvoice> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())
    }

    async fn list_invoices(
        &self,
        tenant_id: Uuid,
        customer_id: Option<Uuid>,
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
        let mut conn = self.get_conn().await?;

        insert_invoice(&mut conn, invoice).await
    }

    async fn insert_invoice_batch(&self, invoice: Vec<InvoiceNew>) -> StoreResult<Vec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let insertable_invoice: Vec<InvoiceRowNew> = invoice
            .into_iter()
            .map(|c| c.try_into())
            .collect::<Result<_, _>>()?;

        let inserted: Vec<Invoice> =
            InvoiceRow::insert_invoice_batch(&mut conn, insertable_invoice)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(|v| {
                    v.into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<Vec<_>, Report<StoreError>>>()
                })?;

        for inv in &inserted {
            process_mrr(inv, &mut conn).await?; // TODO batch
        }

        // TODO update subscription mrr

        Ok(inserted)
    }

    async fn update_invoice_external_status(
        &self,
        invoice_id: Uuid,
        tenant_id: Uuid,
        external_status: InvoiceExternalStatusEnum,
    ) -> StoreResult<()> {
        self.transaction(|conn| {
            async move {
                InvoiceRow::update_external_status(
                    conn,
                    invoice_id,
                    tenant_id,
                    external_status.clone().into(),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                if external_status == InvoiceExternalStatusEnum::Paid {
                    let subscription_id = SubscriptionRow::get_subscription_id_by_invoice_id(
                        conn,
                        &tenant_id,
                        &invoice_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    if let Some(subscription_id) = subscription_id {
                        SubscriptionRow::activate_subscription(conn, subscription_id, tenant_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    process_pending_tx(conn, invoice_id).await?;
                }

                Ok(())
            }
            .scope_boxed()
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

    async fn finalize_invoice(&self, id: Uuid, tenant_id: Uuid) -> StoreResult<()> {
        let patch = compute_invoice_patch(self, id, tenant_id).await?;
        self.transaction(|conn| {
            async move {
                let refreshed = refresh_invoice_data(conn, id, tenant_id, &patch).await?;

                if refreshed.invoice.applied_credits > 0 {
                    CustomerBalance::update(
                        conn,
                        refreshed.customer.id,
                        tenant_id,
                        -refreshed.invoice.applied_credits as i32,
                        Some(refreshed.invoice.id),
                    )
                    .await?;
                }

                InvoiceRow::finalize(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
            }
            .scope_boxed()
        })
        .await?;

        let _ = self
            .eventbus
            .publish(Event::invoice_finalized(id, tenant_id))
            .await;

        Ok(())
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

    async fn refresh_invoice_data(
        &self,
        id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<DetailedInvoice> {
        let patch = compute_invoice_patch(self, id, tenant_id).await?;
        let mut conn = self.get_conn().await?;
        refresh_invoice_data(&mut conn, id, tenant_id, &patch).await
    }

    async fn list_invoices_to_issue(
        &self,
        max_attempts: i32,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let invoices = InvoiceRow::list_to_issue(&mut conn, max_attempts, pagination.into())
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

    async fn invoice_issue_success(&self, id: Uuid, tenant_id: Uuid) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::issue_success(&mut conn, id, tenant_id)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn invoice_issue_error(
        &self,
        id: Uuid,
        tenant_id: Uuid,
        last_issue_error: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::issue_error(&mut conn, id, tenant_id, last_issue_error)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn update_pending_finalization_invoices(&self, now: NaiveDateTime) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::update_pending_finalization(&mut conn, now)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }
}

/*

TODO special cases :
- cancellation/all invoice => all mrr logs after that should be cancelled, unless reactivation
- cancellation : recalculate the mrr delta (as the one in the event was calculated before the invoice was created)
- consolidation if multiple events in the same day. Ex: new business + expansion = new business, or cancellation + reactivation => nothing
 */
async fn process_mrr(inserted: &domain::Invoice, conn: &mut PgConn) -> StoreResult<()> {
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

            let new_log = diesel_models::bi::BiMrrMovementLogRowNew {
                id: Uuid::now_v7(),
                description: description.to_string(),
                movement_type,
                net_mrr_change: mrr_delta,
                currency: inserted.currency.clone(),
                applies_to: inserted.invoice_date,
                invoice_id: inserted.id,
                credit_note_id: None,
                plan_version_id: inserted.plan_version_id.unwrap(), // TODO
                tenant_id: inserted.tenant_id,
            };

            mrr_logs.push(new_log);
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

async fn refresh_invoice_data(
    conn: &mut PgConn,
    id: Uuid,
    tenant_id: Uuid,
    row_patch: &InvoiceRowLinesPatch,
) -> StoreResult<DetailedInvoice> {
    row_patch
        .update_lines(id, tenant_id, conn)
        .await
        .map(|_| ())
        .map_err(Into::<Report<StoreError>>::into)?;

    InvoiceRow::find_by_id(conn, tenant_id, id)
        .await
        .map_err(Into::into)
        .and_then(|row| row.try_into())
}

async fn compute_invoice_patch(
    store: &Store,
    invoice_id: Uuid,
    tenant_id: Uuid,
) -> StoreResult<InvoiceRowLinesPatch> {
    let invoice = store.find_invoice_by_id(tenant_id, invoice_id).await?;

    match invoice.invoice.subscription_id {
        None => Err(StoreError::InvalidArgument(
            "Cannot refresh invoice without subscription_id".into(),
        )
        .into()),
        Some(subscription_id) => {
            let subscription_details = store
                .get_subscription_details(tenant_id, subscription_id)
                .await?;
            let lines = store
                .compute_dated_invoice_lines(&invoice.invoice.invoice_date, subscription_details)
                .await?;

            InvoiceLinesPatch::from_invoice_and_lines(&invoice, lines).try_into()
        }
    }
}

pub async fn insert_invoice(conn: &mut PgConn, invoice: InvoiceNew) -> StoreResult<Invoice> {
    let insertable_invoice: InvoiceRowNew = invoice.try_into()?;

    let inserted: Invoice = insertable_invoice
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .and_then(TryInto::try_into)?;

    process_mrr(&inserted, conn).await?;

    Ok(inserted)
}

async fn process_pending_tx(conn: &mut PgConn, invoice_id: Uuid) -> StoreResult<()> {
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
