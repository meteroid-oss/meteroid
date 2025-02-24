use crate::domain::enums::{InvoiceExternalStatusEnum, InvoiceType};
use crate::errors::StoreError;
use crate::store::Store;
use crate::{domain, StoreResult};
use chrono::NaiveDateTime;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{MrrMovementType, SubscriptionEventType};
use diesel_models::{DbResult, PgConn};
use error_stack::{Report, ResultExt};

use crate::compute::InvoiceLineInterface;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{
    CursorPaginatedVec, CursorPaginationRequest, DetailedInvoice, Invoice, InvoiceLinesPatch,
    InvoiceNew, InvoiceWithCustomer, OrderByRequest, PaginatedVec, PaginationRequest,
};
use crate::repositories::customer_balance::CustomerBalance;
use crate::repositories::SubscriptionInterface;
use crate::utils::decimals::ToUnit;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, TenantId};
use common_eventbus::Event;
use diesel_models::applied_coupons::{AppliedCouponDetailedRow, AppliedCouponRow};
use diesel_models::customer_balance_txs::CustomerBalancePendingTxRow;
use diesel_models::invoices::{InvoiceRow, InvoiceRowLinesPatch, InvoiceRowNew};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::subscriptions::SubscriptionRow;
use tracing_log::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait InvoiceInterface {
    async fn find_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice>;

    async fn list_invoices(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<InvoiceWithCustomer>>;

    async fn insert_invoice(&self, invoice: InvoiceNew) -> StoreResult<Invoice>;

    async fn insert_invoice_batch(&self, invoice: Vec<InvoiceNew>) -> StoreResult<Vec<Invoice>>;

    async fn update_invoice_external_status(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        external_status: InvoiceExternalStatusEnum,
    ) -> StoreResult<()>;

    async fn list_invoices_to_finalize(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn finalize_invoice(&self, id: InvoiceId, tenant_id: TenantId) -> StoreResult<()>;

    async fn list_outdated_invoices(
        &self,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn list_invoices_to_issue(
        &self,
        max_attempts: i32,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<Invoice>>;

    async fn list_invoices_by_ids(&self, ids: Vec<InvoiceId>) -> StoreResult<Vec<Invoice>>;

    async fn invoice_issue_success(&self, id: InvoiceId, tenant_id: TenantId) -> StoreResult<()>;

    async fn invoice_issue_error(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        last_issue_error: &str,
    ) -> StoreResult<()>;

    async fn update_pending_finalization_invoices(&self, now: NaiveDateTime) -> StoreResult<()>;

    async fn refresh_invoice_data(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice>;

    async fn save_invoice_documents(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        pdf_id: String,
        xml_id: Option<String>,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl InvoiceInterface for Store {
    async fn find_invoice_by_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
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

    async fn update_invoice_external_status(
        &self,
        invoice_id: InvoiceId,
        tenant_id: TenantId,
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
                        tenant_id,
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

    async fn finalize_invoice(&self, id: InvoiceId, tenant_id: TenantId) -> StoreResult<()> {
        let patch = compute_invoice_patch(self, id, tenant_id).await?;
        let applied_coupons_amounts = patch.applied_coupons.clone();
        let row_patch = patch.try_into()?;

        self.transaction(|conn| {
            async move {
                let refreshed = refresh_invoice_data(conn, id, tenant_id, &row_patch).await?;
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

                let invoicing_entity = InvoicingEntityRow::select_for_update_by_id_and_tenant(
                    conn,
                    &refreshed.customer.invoicing_entity_id,
                    tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let new_invoice_number = self.internal.format_invoice_number(
                    invoicing_entity.next_invoice_number,
                    invoicing_entity.invoice_number_pattern,
                    refreshed.invoice.invoice_date,
                );

                let applied_coupons_ids =
                    refresh_applied_coupons(conn, &refreshed, &applied_coupons_amounts).await?;

                let res = InvoiceRow::finalize(
                    conn,
                    id,
                    tenant_id,
                    new_invoice_number,
                    &applied_coupons_ids,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                InvoicingEntityRow::update_invoicing_entity_number(
                    conn,
                    &refreshed.customer.invoicing_entity_id,
                    tenant_id,
                    invoicing_entity.next_invoice_number,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let final_invoice: DetailedInvoice = InvoiceRow::find_by_id(conn, tenant_id, id)
                    .await
                    .map_err(Into::into)
                    .and_then(|row| row.try_into())?;

                self.internal
                    .insert_outbox_events_tx(
                        conn,
                        vec![OutboxEvent::invoice_finalized(final_invoice.into())],
                    )
                    .await?;

                Ok(res)
            }
            .scope_boxed()
        })
        .await?;

        let _ = self
            .eventbus
            .publish(Event::invoice_finalized(id.as_uuid(), tenant_id.as_uuid()))
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

    async fn invoice_issue_success(&self, id: InvoiceId, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::issue_success(&mut conn, id, tenant_id)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn invoice_issue_error(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
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

    async fn refresh_invoice_data(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        let patch = compute_invoice_patch(self, id, tenant_id)
            .await?
            .try_into()?;
        let mut conn = self.get_conn().await?;
        refresh_invoice_data(&mut conn, id, tenant_id, &patch).await
    }

    async fn save_invoice_documents(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
        pdf_id: String,
        xml_id: Option<String>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        InvoiceRow::save_invoice_documents(&mut conn, id, tenant_id, pdf_id, xml_id)
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
    id: InvoiceId,
    tenant_id: TenantId,
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
    invoice_id: InvoiceId,
    tenant_id: TenantId,
) -> StoreResult<InvoiceLinesPatch> {
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
                .compute_dated_invoice_lines(&invoice.invoice.invoice_date, &subscription_details)
                .await
                .change_context(StoreError::InvoiceComputationError)?;

            Ok(InvoiceLinesPatch::new(
                &invoice,
                lines,
                &subscription_details.applied_coupons,
            ))
        }
    }
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
    for inv in &inserted {
        process_mrr(inv, tx).await?;
        let final_invoice: DetailedInvoice = InvoiceRow::find_by_id(tx, inv.tenant_id, inv.id)
            .await
            .map_err(Into::into)
            .and_then(|row| row.try_into())?;

        store
            .internal
            .insert_outbox_events_tx(tx, vec![OutboxEvent::invoice_created(final_invoice.into())])
            .await?;
    }

    Ok(inserted)
}

async fn process_pending_tx(conn: &mut PgConn, invoice_id: InvoiceId) -> StoreResult<()> {
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

async fn refresh_applied_coupons(
    tx_conn: &mut PgConn,
    invoice: &DetailedInvoice,
    applied_coupons_amounts: &[(Uuid, i64)],
) -> DbResult<Vec<Uuid>> {
    let applied_coupons_ids: Vec<Uuid> = applied_coupons_amounts.iter().map(|(k, _)| *k).collect();

    let applied_coupons_detailed =
        AppliedCouponDetailedRow::list_by_ids_for_update(tx_conn, &applied_coupons_ids).await?;

    for applied_coupon_detailed in applied_coupons_detailed {
        let amount_delta = if applied_coupon_detailed
            .coupon
            .recurring_value
            .is_some_and(|x| x == 1)
        {
            let cur = rusty_money::iso::find(&invoice.invoice.currency).unwrap();

            applied_coupons_amounts
                .iter()
                .find(|x| x.0 == applied_coupon_detailed.applied_coupon.id)
                .map(|x| x.1.to_unit(cur.exponent as u8))
        } else {
            None
        };

        AppliedCouponRow::refresh_state(
            tx_conn,
            applied_coupon_detailed.applied_coupon.id,
            amount_delta,
        )
        .await?;
    }

    Ok(applied_coupons_ids)
}
