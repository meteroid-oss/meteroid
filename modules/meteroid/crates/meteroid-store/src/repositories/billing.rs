use crate::compute::InvoiceLineInterface;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{
    DetailedInvoice, InlineCustomer, InlineInvoicingEntity, InvoiceNew, InvoiceStatusEnum,
    InvoiceTotals, InvoiceTotalsParams, InvoiceType, InvoicingEntity,
};
use crate::errors::StoreError;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::invoices::insert_invoice_tx;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::{CustomersInterface, SubscriptionInterface};
use crate::store::PgConn;
use crate::Store;
use async_trait::async_trait;
use chrono::NaiveTime;
use common_domain::ids::{
    BaseId, CustomerPaymentMethodId, PaymentTransactionId, SubscriptionId, TenantId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{
    PaymentStatusEnum, PaymentTypeEnum, SubscriptionActivationConditionEnum,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
};
use diesel_models::query::invoicing_entities::get_invoicing_entity_id_by_invoice_id;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, Result, ResultExt};
use stripe_client::payment_intents::{PaymentIntent, StripePaymentStatus};
use tracing::log;

#[async_trait]
pub trait BillingService {
    async fn complete_subscription_checkout(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
    ) -> Result<PaymentTransaction, StoreError>;

    async fn consolidate_transaction(
        &self,
        transaction_id: PaymentTransactionRow,
        payment_intent: PaymentIntent,
    ) -> Result<PaymentTransaction, StoreError>;
}

#[async_trait]
impl BillingService for Store {
    // TODO
    // - check duplicates impact on failure + idempotency
    // - coupons
    // - meaningful error message
    // improve overall tx/conn management
    async fn complete_subscription_checkout(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
    ) -> Result<PaymentTransaction, StoreError> {
        let mut conn = self.get_conn().await?;

        let subscription = self
            .get_subscription_details(tenant_id, subscription_id)
            .await?;

        let currency = subscription.subscription.currency.clone();

        // validate the currency
        if currency != currency_confirmation {
            return Err(Report::new(StoreError::CheckoutError)
                .attach_printable("Currency is different from the confirmation"));
        }

        // TODO skip_usage parameter
        let invoice_lines = self
            .compute_dated_invoice_lines(&subscription.subscription.start_date, &subscription)
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        let customer = self
            .find_customer_by_id(subscription.subscription.customer_id, tenant_id)
            .await?;

        let totals = InvoiceTotals::from_params(InvoiceTotalsParams {
            line_items: &invoice_lines,
            total: 0, // no prepaid
            amount_due: 0,
            tax_rate: 0,
            customer_balance_cents: customer.balance_value_cents,
            subscription_applied_coupons: &vec![], // TODO
            invoice_currency: currency.as_str(),
        });

        // validate the total amount
        if totals.amount_due != (total_amount_confirmation as i64) {
            return Err(Report::new(StoreError::CheckoutError)
                .attach_printable("Total due amount is different from the confirmation"));
        }

        let billing_start_date = subscription
            .subscription
            .billing_start_date
            .unwrap_or(subscription.subscription.start_date);

        let period = crate::utils::periods::calculate_period_range(
            billing_start_date,
            subscription.subscription.billing_day_anchor as u16,
            0,
            &subscription.subscription.period,
        );

        let invoicing_entity = self
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let due_date = (period.end
            + chrono::Duration::days(subscription.subscription.net_terms as i64))
        .and_time(NaiveTime::MIN);
        // In the case of a subscription checkout, the invoice will be archived/cancelled if the transaction doesn't complete
        let invoice_number = "draft";

        let subscription = subscription.subscription;

        let (_invoice, payment_tx) = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let invoice_new = InvoiceNew {
                        tenant_id: subscription.tenant_id,
                        customer_id: subscription.customer_id,
                        subscription_id: Some(subscription.id),
                        plan_version_id: Some(subscription.plan_version_id),
                        invoice_type: InvoiceType::Recurring,
                        currency: subscription.currency.clone(),
                        external_invoice_id: None,
                        line_items: invoice_lines,
                        issued: false,
                        issue_attempts: 0,
                        last_issue_attempt_at: None,
                        last_issue_error: None,
                        data_updated_at: None,
                        status: InvoiceStatusEnum::Draft,
                        external_status: None,
                        invoice_date: period.end,
                        finalized_at: None,
                        total: totals.total,
                        amount_due: totals.amount_due,
                        net_terms: subscription.net_terms as i32,
                        subtotal: totals.subtotal,
                        subtotal_recurring: totals.subtotal_recurring,
                        tax_amount: totals.tax_amount,
                        tax_rate: 0, // TODO
                        reference: None,
                        memo: None,
                        due_at: Some(due_date),
                        plan_name: Some(subscription.plan_name.clone()),
                        invoice_number: invoice_number.to_string(),
                        customer_details: InlineCustomer {
                            id: subscription.customer_id,
                            name: customer.name.clone(),
                            billing_address: customer.billing_address.clone(),
                            vat_number: customer.vat_number.clone(),
                            email: customer.billing_email.clone(),
                            alias: customer.alias.clone(),
                            snapshot_at: chrono::Utc::now().naive_utc(),
                        },
                        seller_details: InlineInvoicingEntity {
                            id: invoicing_entity.id,
                            legal_name: invoicing_entity.legal_name.clone(),
                            vat_number: invoicing_entity.vat_number.clone(),
                            address: invoicing_entity.address(),
                            snapshot_at: chrono::Utc::now().naive_utc(),
                        },
                    };

                    let inserted_invoice = insert_invoice_tx(self, conn, invoice_new).await?;

                    let transaction = PaymentTransactionRowNew {
                        id: PaymentTransactionId::new(),
                        tenant_id,
                        invoice_id: inserted_invoice.id,
                        provider_transaction_id: None,
                        amount: totals.amount_due,
                        currency: subscription.currency.clone(),
                        payment_method_id: Some(payment_method_id),
                        status: PaymentStatusEnum::Pending,
                        payment_type: PaymentTypeEnum::Payment,
                        error_type: None,
                    };

                    let inserted_transaction = transaction
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok((inserted_invoice, inserted_transaction))
                }
                .scope_boxed()
            })
            .await?;

        // we now call the payment provider to create the payment intent
        let payment_intent = self
            .create_payment_intent(
                &tenant_id,
                &payment_method_id,
                &payment_tx.id,
                totals.amount_due as u64,
                currency.clone(),
            )
            .await?;

        // and we consolidate the transaction and invoice status (and subscription status)
        let tx = self
            .consolidate_transaction(payment_tx, payment_intent)
            .await?;

        Ok(tx)
    }

    /// Consolidate the transaction status with the payment intent, and update the related entities (invoice, subscription ?)
    async fn consolidate_transaction(
        &self,
        transaction: PaymentTransactionRow,
        payment_intent: PaymentIntent,
    ) -> Result<PaymentTransaction, StoreError> {
        let mut conn = self.get_conn().await?;

        // Skip processing if the transaction is already in a terminal state
        if transaction.status != PaymentStatusEnum::Pending
            && transaction.status != PaymentStatusEnum::Ready
        {
            log::info!(
                "Transaction {} already in non-pending state: {:?}",
                transaction.id,
                transaction.status
            );
            return Ok(transaction.into());
        }

        // Map Stripe payment status to our internal payment status
        let (new_status, processed_at) = match payment_intent.status {
            StripePaymentStatus::Succeeded => (
                PaymentStatusEnum::Settled,
                Some(chrono::Utc::now().naive_utc()),
            ),
            StripePaymentStatus::Failed => (PaymentStatusEnum::Failed, None),
            StripePaymentStatus::Canceled => (PaymentStatusEnum::Cancelled, None),
            StripePaymentStatus::Pending | StripePaymentStatus::Processing => {
                (PaymentStatusEnum::Pending, None)
            }
            StripePaymentStatus::RequiresCustomerAction
            | StripePaymentStatus::RequiresPaymentMethod
            | StripePaymentStatus::RequiresConfirmation
            | StripePaymentStatus::RequiresCapture => {
                // Customer action is required - keep as Pending but we might want to notify the customer
                log::info!(
                    "Payment intent {} requires customer action: {:?}",
                    payment_intent.id,
                    payment_intent.status
                );
                (PaymentStatusEnum::Pending, None)
            }
            StripePaymentStatus::Chargeable | StripePaymentStatus::Consumed => {
                log::warn!(
                    "Unhandled stripe payment status for transaction {}: {:?}",
                    transaction.id,
                    payment_intent.status
                );
                return Err(
                    Report::new(StoreError::PaymentProviderError).attach_printable(format!(
                        "Unhandled payment status: {:?}",
                        payment_intent.status
                    )),
                );
            }
        };

        // Only update if the status has changed
        if transaction.status == new_status {
            log::debug!(
                "Transaction {} status unchanged: {:?}",
                transaction.id,
                new_status
            );
            return Ok(transaction.into());
        }

        log::info!(
            "Updating transaction {} status from {:?} to {:?}",
            transaction.id,
            transaction.status,
            new_status
        );

        let patch = PaymentTransactionRowPatch {
            id: transaction.id,
            status: Some(new_status.clone()),
            processed_at: Some(processed_at),
            refunded_at: None,
            error_type: Some(payment_intent.last_payment_error),
        };

        let updated_transaction = self
            .transaction_with(&mut conn, |conn| {
                async move {
                    let updated_transaction = patch.update(conn).await?;

                    // Handle post-update actions based on new status
                    match new_status {
                        PaymentStatusEnum::Settled => {
                            // Payment successful - activate subscription or mark invoice as paid
                            self.handle_successful_payment(conn, &updated_transaction)
                                .await?;
                        }
                        PaymentStatusEnum::Failed => {
                            // Payment failed - notify customer or retry?
                            self.handle_failed_payment(conn, &updated_transaction)
                                .await?;
                        }
                        PaymentStatusEnum::Cancelled => {
                            // Payment cancelled - update related entities
                            self.handle_cancelled_payment(conn, &updated_transaction)
                                .await?;
                        }
                        _ => {
                            // No action needed for other statuses
                        }
                    }

                    Ok(updated_transaction)
                }
                .scope_boxed()
            })
            .await?;

        Ok(updated_transaction.into())
    }
}

impl Store {
    async fn handle_successful_payment(
        &self,
        conn: &mut PgConn,
        transaction: &PaymentTransactionRow,
    ) -> Result<(), StoreError> {
        // get invoicing entity id by invoice id

        let invoicing_entity_id = get_invoicing_entity_id_by_invoice_id(
            conn,
            transaction.tenant_id,
            transaction.invoice_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut invoice =
            InvoiceRow::find_by_id(conn, transaction.tenant_id, transaction.invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let subscription_id = invoice.invoice.subscription_id;

        // if the invoice is not finalized nor void, finalize it
        // TODO any action if the invoice is void ?
        if invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Finalized
            && invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Void
        {
            let invoicing_entity: InvoicingEntity =
                InvoicingEntityRow::select_for_update_by_id_and_tenant(
                    conn,
                    invoicing_entity_id,
                    transaction.tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into();

            let invoice_number = self.internal.format_invoice_number(
                invoicing_entity.next_invoice_number,
                invoicing_entity.invoice_number_pattern,
                chrono::Utc::now().naive_utc().date(),
            );

            // TODO coupons
            let _ = InvoiceRow::finalize(
                conn,
                transaction.invoice_id,
                transaction.tenant_id,
                invoice_number,
                &[],
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            InvoicingEntityRow::update_invoicing_entity_number(
                conn,
                invoicing_entity_id,
                transaction.tenant_id,
                invoicing_entity.next_invoice_number,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            invoice = InvoiceRow::find_by_id(conn, transaction.tenant_id, transaction.invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let invoice_domain: DetailedInvoice = invoice.try_into()?;

            // TODO why o2O from row ? ?
            self.internal
                .insert_outbox_events_tx(
                    conn,
                    vec![OutboxEvent::invoice_finalized(invoice_domain.into())],
                )
                .await?;
        }

        // update the invoice amount due to amount_due - transaction.amount. TODO also mark as paid & send an event
        let res = InvoiceRow::apply_transaction(
            conn,
            transaction.invoice_id,
            transaction.tenant_id,
            transaction.amount,
        )
        .await?;

        let completed = res.amount_due == 0;

        if completed {
            if let Some(subscription_id) = subscription_id.as_ref() {
                let subscription = SubscriptionRow::get_subscription_by_id(
                    conn,
                    &transaction.tenant_id,
                    *subscription_id,
                )
                .await?;

                // TODO also in case of paused / ex: because of failed payments
                let should_activate = subscription.subscription.activated_at.is_none()
                    && subscription.subscription.activation_condition
                        == SubscriptionActivationConditionEnum::OnCheckout;
                if should_activate {
                    // TODO send an event
                    SubscriptionRow::activate_subscription(
                        conn,
                        subscription_id,
                        &transaction.tenant_id,
                    )
                    .await?;
                }
            }
        }

        // TODO payment confirmation to customer
        Ok(())
    }

    async fn handle_failed_payment(
        &self,
        _conn: &mut PgConn,
        _transaction: &PaymentTransactionRow,
    ) -> Result<(), StoreError> {
        // (notifying the customer etc => TODO controller)
        // maybe delete the invoice if it's checkout
        // Automated payment should retry asynchronously
        Ok(())
    }

    // Handle cancelled payment
    async fn handle_cancelled_payment(
        &self,
        _conn: &mut PgConn,
        _transaction: &PaymentTransactionRow,
    ) -> Result<(), StoreError> {
        // maybe delete the invoice if it's checkout
        // Automated payment should retry asynchronously
        Ok(())
    }
}
