use crate::StoreResult;
use crate::domain::{
    Invoice, InvoiceLinesPatch, InvoiceTotals, InvoiceTotalsParams, SubscriptionDetails,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use common_domain::ids::{InvoiceId, TenantId};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::PgConn;
use diesel_models::invoices::{InvoiceRow, InvoiceRowLinesPatch};
use error_stack::{Report, ResultExt, bail};

impl Services {
    pub(in crate::services) async fn refresh_invoice_data(
        &self,
        conn: &mut PgConn,
        id: InvoiceId,
        tenant_id: TenantId,
        subscription_details_cache: &Option<SubscriptionDetails>,
        refresh_invoice_lines: bool,
    ) -> StoreResult<Invoice> {
        let invoice_details = self
            .store
            .transaction_with(conn, |conn| {
                async move {
                    let invoice_lock =
                        InvoiceRow::select_for_update_by_id(conn, tenant_id, id).await?;

                    let invoice: Invoice = invoice_lock.invoice.try_into()?;

                    if !invoice.can_edit() {
                        bail!(StoreError::InvalidArgument(
                            "Cannot refresh invoice that is not in draft or pending status".into(),
                        ));
                    }

                    let patch = self
                        .build_invoice_lines_patch(
                            conn,
                            &invoice,
                            invoice_lock.customer_balance,
                            subscription_details_cache,
                            refresh_invoice_lines,
                        )
                        .await?;
                    let row_patch: InvoiceRowLinesPatch = patch.try_into()?;

                    row_patch
                        .update_lines(id, tenant_id, conn)
                        .await
                        .map(|_| ())
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok::<_, StoreErrorReport>(invoice)
                }
                .scope_boxed()
            })
            .await?;

        Ok(invoice_details)
    }

    /// To be called alongside the other refresh, like the coupons (and mrr?)
    pub(in crate::services) async fn build_invoice_lines_patch(
        &self,
        conn: &mut PgConn,
        invoice: &Invoice,
        customer_balance: i64,
        subscription_details_cache: &Option<SubscriptionDetails>,
        refresh_invoice_lines: bool,
    ) -> StoreResult<InvoiceLinesPatch> {
        if !invoice.can_edit() {
            bail!(StoreError::InvalidArgument(
                "Cannot refresh invoice that is not in draft or pending status".into(),
            ));
        }

        let lines = if refresh_invoice_lines {
            let res;

            let subscription_details = match subscription_details_cache {
                Some(details) => details,
                None => match invoice.subscription_id {
                    Some(subscription_id) => {
                        res = self
                            .store
                            .get_subscription_details_with_conn(
                                conn,
                                invoice.tenant_id,
                                subscription_id,
                            )
                            .await?;
                        &res
                    }
                    None => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot refresh invoice lines without subscription details".into(),
                        )
                        .into());
                    }
                },
            };

            self.compute_invoice_lines(conn, &invoice.invoice_date, subscription_details)
                .await
                .change_context(StoreError::InvoiceComputationError)?
        } else {
            invoice.line_items.clone()
        };

        let totals = InvoiceTotals::from_params(InvoiceTotalsParams {
            line_items: &lines,
            total: invoice.total,
            amount_due: invoice.amount_due,
            tax_rate: invoice.tax_rate,
            customer_balance_cents: customer_balance,
            subscription_applied_coupons: subscription_details_cache
                .as_ref()
                .map_or(&Vec::new(), |a| &a.applied_coupons), // TODO allow coupons in one-off invoices, also when no subscription_details_cache is provided
            invoice_currency: invoice.currency.as_str(),
        });

        Ok(InvoiceLinesPatch {
            line_items: lines,
            amount_due: totals.amount_due,
            subtotal: totals.subtotal,
            subtotal_recurring: totals.subtotal_recurring,
            total: totals.total,
            tax_amount: totals.tax_amount,
            applied_credits: totals.applied_credits,
            applied_coupons: totals.applied_coupons,
        })
    }
}
