use diesel::QueryDsl;
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::{Report, ResultExt};
use common_domain::ids::TenantId;
use diesel_models::enums::SubscriptionActivationConditionEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscriptions::SubscriptionRow;
use crate::{ StoreResult};
use crate::domain::{Invoice, InvoicePaymentStatus, ResolvedPaymentMethod};
use crate::domain::outbox_event::{InvoiceEvent, InvoicePdfGeneratedEvent, OutboxEvent, PaymentTransactionEvent};
use crate::errors::StoreError;
use crate::repositories::outbox::OutboxInterface;
use crate::services::Services;

impl Services {

pub async fn on_payment_transaction_settled(
    &self,
    event: PaymentTransactionEvent
) -> StoreResult<()> {

    self.store.transaction(
        |conn| async move {
            let invoice = InvoiceRow::select_for_update_by_id(
                conn,
                event.tenant_id,
                event.invoice_id,
            )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let subscription_id = invoice.invoice.subscription_id;

            let should_finalize =  invoice.invoice.status == diesel_models::enums::InvoiceStatusEnum::Draft;

            // if the invoice is not finalized nor void, finalize it (no line data refresh)
            if should_finalize {
                self.finalize_invoice_tx(
                    conn,
                    invoice.invoice.id,
                    invoice.invoice.tenant_id,
                    false,
                    &None,
                )
                    .await?;
            }

            // update the invoice amount due to amount_due - transaction.amount
            let res = InvoiceRow::apply_transaction(
                conn,
                event.invoice_id,
                event.tenant_id,
                event.amount,
            )
                .await?;


            let completed = res.amount_due == 0;

            if completed {
                let invoice: Invoice = res.try_into()?;

                InvoiceRow::apply_payment_status(
                    conn,
                    event.invoice_id,
                    event.tenant_id,
                    diesel_models::enums::InvoicePaymentStatus::Paid,
                )
                    .await?;

                self.store.insert_outbox_event_tx(
                    conn,
                    OutboxEvent::invoice_paid((&invoice).into())
                ).await?;

                if let Some(subscription_id) = subscription_id.as_ref() {
                    let subscription = SubscriptionRow::get_subscription_by_id(
                        conn,
                        &event.tenant_id,
                        *subscription_id,
                    )
                        .await?;

                    // Activate subscription if needed
                    let should_activate = subscription.subscription.activated_at.is_none()
                        && subscription.subscription.activation_condition == SubscriptionActivationConditionEnum::OnCheckout;
                    if should_activate {
                        // TODO send a subscription_activated event
                        SubscriptionRow::activate_subscription(
                            conn,
                            subscription_id,
                            &event.tenant_id,
                        )
                            .await?;
                    }
                }
            } else {
                InvoiceRow::apply_payment_status(
                    conn,
                    event.invoice_id,
                    event.tenant_id,
                    diesel_models::enums::InvoicePaymentStatus::PartiallyPaid,
                )
                    .await?;
            }

            // TODO payment receipt


            Ok(())

        }.scope_boxed()
    )
        .await ?;

    Ok(())

}

}
