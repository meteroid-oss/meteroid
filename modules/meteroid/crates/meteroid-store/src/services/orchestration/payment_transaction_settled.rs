use crate::StoreResult;
use crate::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use crate::domain::{Invoice, InvoicePaymentStatus, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::outbox::OutboxInterface;
use crate::services::Services;
use crate::utils::periods::calculate_advance_period_range;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{CycleActionEnum, SubscriptionActivationConditionEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;

impl Services {
    pub async fn on_payment_transaction_settled(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(
                        conn,
                        event.tenant_id,
                        event.invoice_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let subscription_id = invoice.invoice.subscription_id;

                    let should_finalize =
                        invoice.invoice.status == diesel_models::enums::InvoiceStatusEnum::Draft;

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

                        if invoice.payment_status != InvoicePaymentStatus::Paid {
                            InvoiceRow::apply_payment_status(
                                conn,
                                event.invoice_id,
                                event.tenant_id,
                                diesel_models::enums::InvoicePaymentStatus::Paid,
                                event.processed_at,
                            )
                            .await?;

                            self.store
                                .insert_outbox_event_tx(
                                    conn,
                                    OutboxEvent::invoice_paid((&invoice).into()),
                                )
                                .await?;
                        }

                        // todo should this be on_invoice_paid?
                        if let Some(subscription_id) = subscription_id.as_ref() {
                            let subscription = SubscriptionRow::get_subscription_by_id(
                                conn,
                                &event.tenant_id,
                                *subscription_id,
                            )
                            .await?;

                            // Activate subscription if needed
                            let should_activate = subscription.subscription.activated_at.is_none()
                                && subscription.subscription.activation_condition
                                    == SubscriptionActivationConditionEnum::OnCheckout;
                            if should_activate {
                                let billing_start_date = subscription
                                    .subscription
                                    .billing_start_date
                                    .unwrap_or(chrono::Utc::now().date_naive());

                                let current_period_start;
                                let current_period_end;
                                let next_cycle_action;
                                let mut cycle_index = None;
                                let status;

                                if subscription.subscription.trial_duration.is_some() {
                                    status = SubscriptionStatusEnum::TrialActive;
                                    current_period_start = billing_start_date;
                                    current_period_end = Some(
                                        current_period_start
                                            + chrono::Duration::days(i64::from(
                                                subscription.subscription.trial_duration.unwrap(),
                                            )),
                                    );
                                    next_cycle_action = Some(CycleActionEnum::EndTrial);
                                } else {
                                    let range = calculate_advance_period_range(
                                        billing_start_date,
                                        subscription.subscription.billing_day_anchor as u32,
                                        true,
                                        &subscription.subscription.period.into(),
                                    );

                                    status = SubscriptionStatusEnum::Active;
                                    cycle_index = Some(0);
                                    current_period_start = range.start;
                                    current_period_end = Some(range.end);
                                    next_cycle_action = Some(CycleActionEnum::RenewSubscription);
                                }

                                // TODO send a subscription_activated event
                                SubscriptionRow::activate_subscription(
                                    conn,
                                    subscription_id,
                                    &event.tenant_id,
                                    current_period_start,
                                    current_period_end,
                                    next_cycle_action,
                                    cycle_index,
                                    status.into(),
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
                            event.processed_at,
                        )
                        .await?;
                    }

                    // TODO payment receipt

                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        Ok(())
    }
}
