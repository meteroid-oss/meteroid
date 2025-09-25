use crate::StoreResult;
use crate::domain::{Invoice, InvoiceLinesPatch, SubscriptionDetails};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
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

        let content = if refresh_invoice_lines {
            let mut res;

            let subscription_details = match subscription_details_cache {
                Some(details) => {
                    if customer_balance != details.customer.balance_value_cents {
                        res = details.clone();
                        res.customer.balance_value_cents = customer_balance;
                        &res
                    } else {
                        details
                    }
                }
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

            let already_paid_amount = invoice.total - invoice.amount_due;

            self.compute_invoice(
                conn,
                &invoice.invoice_date,
                subscription_details,
                if already_paid_amount > 0 {
                    Some(already_paid_amount as u64)
                } else {
                    None
                },
                Some(invoice),
            )
            .await
            .change_context(StoreError::InvoiceComputationError)?
        } else {
            ComputedInvoiceContent {
                invoice_lines: invoice.line_items.clone(),
                applied_coupons: invoice.coupons.clone(),
                discount: invoice.discount,
                total: invoice.total,
                amount_due: invoice.amount_due,
                subtotal: invoice.subtotal,
                subtotal_recurring: invoice.subtotal_recurring,
                tax_amount: invoice.tax_amount,
                applied_credits: invoice.applied_credits,
                tax_breakdown: invoice.tax_breakdown.clone(),
            }
        };

        Ok(InvoiceLinesPatch {
            line_items: content.invoice_lines,
            amount_due: content.amount_due,
            subtotal: content.subtotal,
            subtotal_recurring: content.subtotal_recurring,
            total: content.total,
            tax_amount: content.tax_amount,
            applied_credits: content.applied_credits,
            applied_coupons: content.applied_coupons,
            tax_breakdown: content.tax_breakdown,
        })
    }
}
