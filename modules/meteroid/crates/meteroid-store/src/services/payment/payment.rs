use crate::StoreResult;
use crate::adapters::payment_service_providers::initialize_payment_provider;
use crate::domain::connectors::Connector;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{
    BaseId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{
    PaymentStatusEnum, PaymentTypeEnum, SubscriptionActivationConditionEnum,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::{Report, ResultExt};
use stripe_client::payment_intents::{PaymentIntent, StripePaymentStatus};

impl Services {
    /// Creates a payment intent and the associated payment transaction.
    pub(in crate::services) async fn process_invoice_payment_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
    ) -> StoreResult<PaymentTransaction> {
        // Get the invoice
        let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Allow both draft and finalized invoices
        if invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Draft
            && invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Finalized
        {
            return Err(Report::new(StoreError::BillingError)
                .attach_printable("Cannot process payment for this invoice status"));
        }

        if invoice.invoice.amount_due <= 0 {
            return Err(
                Report::new(StoreError::BillingError).attach_printable("Invoice has no amount due")
            );
        }

        // Create a payment transaction
        let transaction = PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id,
            invoice_id,
            provider_transaction_id: None,
            amount: invoice.invoice.amount_due,
            currency: invoice.invoice.currency.clone(),
            payment_method_id: Some(payment_method_id),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
        };

        let inserted_transaction = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Create payment intent with payment provider
        let payment_intent = self
            .create_payment_intent(
                conn,
                &tenant_id,
                &payment_method_id,
                &inserted_transaction.id,
                inserted_transaction.amount as u64,
                inserted_transaction.currency.clone(),
            )
            .await?;

        // Consolidate the transaction
        let tx = self
            .consolidate_transaction(conn, inserted_transaction, payment_intent)
            .await?;

        Ok(tx)
    }

    async fn create_payment_intent(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        payment_method_id: &CustomerPaymentMethodId,
        transaction_id: &PaymentTransactionId,
        amount: u64,
        currency: String,
    ) -> StoreResult<PaymentIntent> {
        let method = CustomerPaymentMethodRow::get_by_id(conn, tenant_id, payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;

        let provider = initialize_payment_provider(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let payment_intent = provider
            .create_payment_intent_in_provider(
                &connector,
                transaction_id,
                &connection.external_customer_id,
                &method.external_payment_method_id,
                amount as i64,
                &currency,
            )
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        Ok(payment_intent)
    }

    async fn consolidate_transaction(
        &self,
        conn: &mut PgConn,
        transaction: PaymentTransactionRow,
        payment_intent: PaymentIntent,
    ) -> error_stack::Result<PaymentTransaction, StoreError> {
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
                tracing::log::info!(
                    "Payment intent {} requires customer action: {:?}",
                    payment_intent.id,
                    payment_intent.status
                );
                (PaymentStatusEnum::Pending, None)
            }
            StripePaymentStatus::Chargeable | StripePaymentStatus::Consumed => {
                tracing::log::warn!(
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
            .store
            .transaction_with(conn, |conn| {
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

    async fn handle_successful_payment(
        &self,
        conn: &mut PgConn,
        transaction: &PaymentTransactionRow,
    ) -> error_stack::Result<(), StoreError> {
        // get invoicing entity id by invoice id

        let invoice = InvoiceRow::select_for_update_by_id(
            conn,
            transaction.tenant_id,
            transaction.invoice_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let subscription_id = invoice.invoice.subscription_id;

        let should_finalize = transaction.status == PaymentStatusEnum::Settled
            && (invoice.invoice.status == diesel_models::enums::InvoiceStatusEnum::Draft
                || invoice.invoice.status == diesel_models::enums::InvoiceStatusEnum::Pending);

        // if the invoice is not finalized nor void, finalize it (no line data refresh)
        if should_finalize {
            self.store
                .transaction_with(conn, |conn| {
                    async move {
                        self.finalize_invoice_tx(
                            conn,
                            invoice.invoice.id,
                            invoice.invoice.tenant_id,
                            false,
                            &None,
                        )
                        .await
                    }
                    .scope_boxed()
                })
                .await?;
        }

        // update the invoice amount due to amount_due - transaction.amount
        let res = InvoiceRow::apply_transaction(
            conn,
            transaction.invoice_id,
            transaction.tenant_id,
            transaction.amount,
        )
        .await?;

        let completed = res.amount_due == 0;

        if completed {
            // invocie paid outbox event

            if let Some(subscription_id) = subscription_id.as_ref() {
                let subscription = SubscriptionRow::get_subscription_by_id(
                    conn,
                    &transaction.tenant_id,
                    *subscription_id,
                )
                .await?;

                // Activate subscription if needed
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

    // Existing methods for handling failed/cancelled payments
    async fn handle_failed_payment(
        &self,
        _conn: &mut PgConn,
        _transaction: &PaymentTransactionRow,
    ) -> error_stack::Result<(), StoreError> {
        // (notifying the customer etc => TODO controller)
        // maybe delete the invoice if it's draft(checkout) TODO
        // Automated payment should retry asynchronously
        Ok(())
    }

    async fn handle_cancelled_payment(
        &self,
        _conn: &mut PgConn,
        _transaction: &PaymentTransactionRow,
    ) -> error_stack::Result<(), StoreError> {
        // maybe delete the invoice if it's checkout
        // Automated payment should retry asynchronously
        Ok(())
    }
}
