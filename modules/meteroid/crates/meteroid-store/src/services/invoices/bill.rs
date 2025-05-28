use crate::StoreResult;
use crate::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use crate::domain::{Customer, DetailedInvoice, Invoice, PaymentStatusEnum, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::store::PgConn;
use chrono::NaiveTime;
use common_domain::ids::{CustomerPaymentMethodId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::customers::CustomerRow;
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;

pub enum InvoiceBillingMode {
    /// Post checkout. We request a payment and don't finalize the invoice until paid.
    FinalizeAfterPayment {
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
    },
    /// Subscription renewal or terminated. If grace period, we schedule finalization. Else, we immediately finalize and request payment.
    AwaitGracePeriodIfApplicable,
    /// Subscription created without checkout (ex: upgrade/downgrade). We immediately finalize and request payment.
    Immediate,
}

impl Services {
    pub(in crate::services) async fn bill_subscription_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        mode: InvoiceBillingMode,
    ) -> StoreResult<Option<DetailedInvoice>> {
        // TODO also check isFree for faster path

        let subscription = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;

        let customer =
            CustomerRow::find_by_id(conn, &subscription.subscription.customer_id, &tenant_id)
                .await
                .map_err(Into::into)
                .and_then(TryInto::try_into)?;

        self.bill_subscription_with_data_tx(conn, tenant_id, subscription, customer, mode)
            .await
    }

    pub(in crate::services) async fn bill_subscription_with_data_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription: SubscriptionDetails,
        customer: Customer,
        mode: InvoiceBillingMode,
    ) -> StoreResult<Option<DetailedInvoice>> {
        let draft_invoice = self
            .create_subscription_draft_invoice(
                conn,
                tenant_id,
                &subscription,
                customer.clone(), /* TODO */
            )
            .await?;

        let draft_invoice = match draft_invoice {
            Some(invoice) => invoice,
            None => {
                log::info!(
                    "No draft invoice created for subscription {}. Skipping billing.",
                    subscription.subscription.id
                );
                return Ok(None);
            }
        };

        match mode {
            InvoiceBillingMode::FinalizeAfterPayment {
                currency_confirmation,
                total_amount_confirmation,
                payment_method_id,
            } => {
                if draft_invoice.currency != currency_confirmation {
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach_printable("Currency is different from the confirmation"));
                }

                // Validate the total amount
                if draft_invoice.amount_due != (total_amount_confirmation as i64) {
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach_printable("Total due amount is different from the confirmation"));
                }

                // We trigger the payment but don't finalize the invoice yet, it will be done via the webhook
                let res = self
                    .process_invoice_payment_tx(
                        conn,
                        tenant_id,
                        draft_invoice.id,
                        payment_method_id,
                    )
                    .await?;

                if res.status == PaymentStatusEnum::Settled {
                    // we finalize the invoice directly
                    self.finalize_invoice_tx(
                        conn,
                        draft_invoice.id,
                        tenant_id,
                        false, // no need to refresh lines, we just paid
                        &Some(subscription),
                    )
                    .await?;
                } else {
                    // we return the draft invoice, it will be finalized later via the webhook
                    return self.as_detailed_invoice(draft_invoice, customer).map(Some);
                }
            }
            InvoiceBillingMode::AwaitGracePeriodIfApplicable => {
                let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                    conn,
                    subscription.subscription.invoicing_entity_id,
                    tenant_id,
                )
                .await?;

                if invoicing_entity.grace_period_hours >= 0 {
                    // Schedule finalization after grace period
                    self.schedule_invoice_finalization(
                        conn,
                        tenant_id,
                        &subscription,
                        draft_invoice.id,
                        draft_invoice.invoice_date,
                        invoicing_entity.grace_period_hours,
                    )
                    .await?;

                    return self.as_detailed_invoice(draft_invoice, customer).map(Some);
                }

                // else we finalize immediately, and trigger payment
                self.finalize_and_process_payment(conn, tenant_id, draft_invoice.id, &subscription)
                    .await?;
            }
            InvoiceBillingMode::Immediate => {
                // Finalize and process payment immediately
                self.finalize_and_process_payment(conn, tenant_id, draft_invoice.id, &subscription)
                    .await?;
            }
        };

        // Get the updated invoice after payment processing
        let updated_invoice =
            InvoiceRow::find_detailed_by_id(conn, tenant_id, draft_invoice.id).await?;

        Ok(DetailedInvoice::try_from(updated_invoice)?).map(Some)
    }

    fn as_detailed_invoice(
        &self,
        invoice: Invoice,
        customer: Customer,
    ) -> StoreResult<DetailedInvoice> {
        Ok(DetailedInvoice {
            invoice,
            plan: None, // TODO
            customer,
            transactions: vec![],
        })
    }

    /// Finalize invoice and process payment
    async fn finalize_and_process_payment(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        subscription: &SubscriptionDetails,
    ) -> StoreResult<()> {
        self.finalize_invoice_tx(
            conn,
            invoice_id,
            tenant_id,
            false,
            &Some(subscription.clone()),
        )
        .await?;

        let subscription_payment_method = SubscriptionRow::get_subscription_payment_method_by_id(
            conn,
            &tenant_id,
            subscription.subscription.id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        if let Some(payment_method) = subscription_payment_method {
            self.process_invoice_payment_tx(conn, tenant_id, invoice_id, payment_method)
                .await?;
        }

        Ok(())
    }

    /// Schedule invoice finalization after grace period
    async fn schedule_invoice_finalization(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription: &SubscriptionDetails,
        invoice_id: InvoiceId,
        invoice_date: chrono::NaiveDate,
        grace_period_hours: i32,
    ) -> StoreResult<()> {
        let scheduled_time = invoice_date.and_time(NaiveTime::MIN)
            + chrono::Duration::hours(grace_period_hours as i64);

        self.store
            .schedule_events(
                conn,
                vec![ScheduledEventNew {
                    subscription_id: subscription.subscription.id,
                    tenant_id,
                    scheduled_time,
                    event_data: ScheduledEventData::FinalizeInvoice { invoice_id },
                    source: "".to_string(),
                }],
            )
            .await?;

        Ok(())
    }
}
