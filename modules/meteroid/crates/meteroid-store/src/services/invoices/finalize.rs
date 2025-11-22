use crate::StoreResult;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{CouponLineItem, DetailedInvoice, Invoice, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::customer_balance::CustomerBalance;
use crate::services::Services;
use crate::services::utils::format_invoice_number;
use common_domain::ids::{AppliedCouponId, BaseId, InvoiceId, InvoicingEntityId, TenantId};
use common_eventbus::Event;
use common_utils::decimals::ToUnit;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::applied_coupons::{AppliedCouponDetailedRow, AppliedCouponRow};
use diesel_models::invoices::{InvoiceRow, InvoiceRowLinesPatch};
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::{DbResult, PgConn};
use error_stack::Report;

impl Services {
    pub async fn finalize_invoice(
        &self,
        id: InvoiceId,
        tenant_id: TenantId,
    ) -> StoreResult<DetailedInvoice> {
        self.store
            .transaction(|conn| {
                self.finalize_invoice_tx(conn, id, tenant_id, false, &None)
                    .scope_boxed()
            })
            .await
    }

    /// Mark an invoice as finalized, incrementing the invoice number counter and applying attached coupons
    pub async fn finalize_invoice_tx(
        &self,
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        refresh_invoice_lines: bool,
        subscription_details_for_refresh: &Option<SubscriptionDetails>,
    ) -> StoreResult<DetailedInvoice> {
        let invoice_lock = InvoiceRow::select_for_update_by_id(conn, tenant_id, id).await?;

        let invoice: Invoice = invoice_lock.invoice.try_into()?;

        let patch = self
            .build_invoice_lines_patch(
                conn,
                &invoice,
                invoice_lock.customer_balance,
                subscription_details_for_refresh,
                refresh_invoice_lines,
            )
            .await?;
        let applied_coupons_amounts = patch.applied_coupons.clone();
        let row_patch: InvoiceRowLinesPatch = patch.try_into()?;

        row_patch
            .update_lines(id, tenant_id, conn)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)?;

        if row_patch.applied_credits > 0 {
            CustomerBalance::update(
                conn,
                invoice.customer_id,
                tenant_id,
                -row_patch.applied_credits,
                Some(id),
            )
            .await?;
        }

        let invoice_details = self
            .increment_and_finalize(
                conn,
                invoice,
                invoice_lock.customer_invoicing_entity_id,
                applied_coupons_amounts,
            )
            .await?;

        Ok(invoice_details)
    }

    async fn increment_and_finalize(
        &self,
        tx: &mut PgConn,
        invoice: Invoice,
        invoicing_entity_id: InvoicingEntityId,
        applied_coupons_amounts: Vec<CouponLineItem>,
    ) -> StoreResult<DetailedInvoice> {
        let invoicing_entity = InvoicingEntityRow::select_for_update_by_id_and_tenant(
            tx,
            invoicing_entity_id,
            invoice.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let new_invoice_number = format_invoice_number(
            invoicing_entity.next_invoice_number,
            invoicing_entity.invoice_number_pattern,
            invoice.invoice_date,
        );

        // Generate payment reference by removing special characters for easier manual entry
        let payment_reference = new_invoice_number
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();

        let _ = refresh_applied_coupons(tx, &invoice.currency, &applied_coupons_amounts).await?;

        let applied_coupons_json = serde_json::to_value(&applied_coupons_amounts)
            .map_err(|e| StoreError::SerdeError("Failed to serialize coupons".to_string(), e))?;

        InvoiceRow::finalize(
            tx,
            invoice.id,
            invoice.tenant_id,
            new_invoice_number,
            payment_reference,
            applied_coupons_json,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        InvoicingEntityRow::update_invoicing_entity_number(
            tx,
            invoicing_entity_id,
            invoice.tenant_id,
            invoicing_entity.next_invoice_number,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let final_invoice: DetailedInvoice =
            InvoiceRow::find_detailed_by_id(tx, invoice.tenant_id, invoice.id)
                .await
                .map_err(Into::into)
                .and_then(std::convert::TryInto::try_into)?;

        let invoice_event = (&final_invoice.invoice).into();
        self.store
            .internal
            .insert_outbox_events_tx(tx, vec![OutboxEvent::invoice_finalized(invoice_event)])
            .await?;

        let _ = self
            .store
            .eventbus
            .publish(Event::invoice_finalized(
                invoice.id.as_uuid(),
                invoice.tenant_id.as_uuid(),
            ))
            .await;

        Ok(final_invoice)
    }
}

async fn refresh_applied_coupons(
    tx_conn: &mut PgConn,
    currency: &str,
    applied_coupons_amounts: &[CouponLineItem],
) -> DbResult<Vec<AppliedCouponId>> {
    let applied_coupons_ids: Vec<AppliedCouponId> = applied_coupons_amounts
        .iter()
        .map(|c| c.applied_coupon_id)
        .collect();

    let applied_coupons_detailed =
        AppliedCouponDetailedRow::list_by_ids_for_update(tx_conn, &applied_coupons_ids).await?;

    for applied_coupon_detailed in applied_coupons_detailed {
        let amount_delta = if applied_coupon_detailed
            .coupon
            .recurring_value
            .is_some_and(|x| x >= 1)
        {
            let cur = rusty_money::iso::find(currency).unwrap();

            applied_coupons_amounts
                .iter()
                .find(|x| x.applied_coupon_id == applied_coupon_detailed.applied_coupon.id)
                .map(|x| x.value.to_unit(cur.exponent as u8))
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
