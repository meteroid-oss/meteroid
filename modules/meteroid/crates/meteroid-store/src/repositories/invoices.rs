use crate::domain::enums::{InvoiceStatusEnum, InvoiceType};
use crate::errors::StoreError;
use crate::store::Store;
use crate::{domain, StoreResult};
use diesel_models::enums::{MrrMovementType, SubscriptionEventType};
use diesel_models::PgConn;
use error_stack::Report;

use crate::domain::{
    Customer, Invoice, InvoiceWithPlanDetails, OrderByRequest, PaginatedVec, PaginationRequest,
};
use tracing_log::log;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait InvoiceInterface {
    async fn find_invoice_by_id(
        &self,
        tenant_id: Uuid,
        invoice_id: Uuid,
    ) -> StoreResult<domain::InvoiceWithPlanDetails>;

    async fn list_invoices(
        &self,
        tenant_id: Uuid,
        customer_id: Option<Uuid>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<domain::Invoice>>;

    async fn insert_invoice(&self, invoice: domain::InvoiceNew) -> StoreResult<domain::Invoice>;

    async fn insert_invoice_batch(
        &self,
        invoice: Vec<domain::InvoiceNew>,
    ) -> StoreResult<Vec<domain::Invoice>>;
}

#[async_trait::async_trait]
impl InvoiceInterface for Store {
    async fn find_invoice_by_id(
        &self,
        tenant_id: Uuid,
        invoice_id: Uuid,
    ) -> StoreResult<InvoiceWithPlanDetails> {
        let mut conn = self.get_conn().await?;

        diesel_models::invoices::Invoice::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn list_invoices(
        &self,
        tenant_id: Uuid,
        customer_id: Option<Uuid>,
        status: Option<domain::enums::InvoiceStatusEnum>,
        query: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Invoice>> {
        let mut conn = self.get_conn().await?;

        let rows = diesel_models::invoices::Invoice::list(
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

        let res: PaginatedVec<Invoice> = PaginatedVec {
            items: rows.items.into_iter().map(|s| s.into()).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn insert_invoice(&self, invoice: domain::InvoiceNew) -> StoreResult<domain::Invoice> {
        let mut conn = self.get_conn().await?;

        let insertable_invoice: diesel_models::invoices::InvoiceNew = invoice.into();

        let inserted: domain::Invoice = insertable_invoice
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(Into::into)?;

        process_mrr(&inserted, &mut conn).await?;

        Ok(inserted)
    }

    async fn insert_invoice_batch(
        &self,
        invoice: Vec<domain::InvoiceNew>,
    ) -> StoreResult<Vec<domain::Invoice>> {
        let mut conn = self.get_conn().await?;

        let insertable_invoice: Vec<diesel_models::invoices::InvoiceNew> =
            invoice.into_iter().map(|c| c.into()).collect();

        let inserted: Vec<domain::Invoice> =
            diesel_models::invoices::Invoice::insert_invoice_batch(&mut conn, insertable_invoice)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .map(|v| v.into_iter().map(Into::into).collect())?;

        for inv in &inserted {
            process_mrr(inv, &mut conn).await?; // TODO batch
        }

        // TODO update subscription mrr

        Ok(inserted)
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
        let subscription_events = diesel_models::subscription_events::SubscriptionEvent::fetch_by_subscription_id_and_date(
            conn,
            inserted.subscription_id,
            inserted.invoice_date,
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        log::info!("subscription_events len {}", subscription_events.len());

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

            let new_log = diesel_models::bi::BiMrrMovementLogNew {
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

        diesel_models::bi::BiMrrMovementLog::insert_movement_log_batch(conn, mrr_logs)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        diesel_models::subscriptions::Subscription::update_subscription_mrr_delta(
            conn,
            inserted.subscription_id,
            mrr_delta_cents,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok(())
}
