use crate::StoreResult;
use crate::adapters::payment::bridge::payment_intent_from_outcome;
use crate::adapters::payment::initialize_payment_connector;
use crate::adapters::payment::model::{ChargeRequest, IdempotencyKey};
use crate::domain::connectors::Connector;
use crate::domain::entity_activity::Actor;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
use crate::errors::StoreError;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{
    BaseId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use error_stack::{Report, ResultExt};
use std::time::Duration;

/// Maximum time to wait for payment provider API calls.
/// This is a safety net in addition to the HTTP client timeout.
const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

impl Services {
    /// Creates a payment intent and the associated payment transaction.
    pub(in crate::services) async fn process_invoice_payment_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        on_session: bool,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        // Get the invoice
        let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // A consolidated child is billed via its parent; paying it directly would double-charge.
        if invoice.invoice.consolidated_into_invoice_id.is_some() {
            return Err(Report::new(StoreError::BillingError)
                .attach("Cannot pay an invoice merged into a consolidated parent"));
        }

        // Allow both draft and finalized invoices
        if invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Draft
            && invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Finalized
        {
            return Err(Report::new(StoreError::BillingError)
                .attach("Cannot process payment for this invoice status"));
        }

        if invoice.invoice.amount_due <= 0 {
            return Err(Report::new(StoreError::BillingError).attach("Invoice has no amount due"));
        }

        // Check for existing transactions that would prevent a new payment
        let existing_transactions =
            PaymentTransactionRow::list_by_invoice_id(conn, invoice_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        // Check for pending transactions - only one payment attempt at a time
        let has_pending_transaction = existing_transactions
            .iter()
            .any(|tx| tx.transaction.status == PaymentStatusEnum::Pending);

        if has_pending_transaction {
            return Err(Report::new(StoreError::PaymentError(
                "A payment for this invoice is already being processed. Please wait for it to complete before attempting another payment.".to_string()
            )));
        }

        // Calculate total of active payments (pending/ready/settled) to prevent over-payment.
        // This check, combined with SELECT FOR UPDATE on the invoice, ensures atomicity
        // in a distributed environment.
        let active_payment_sum: i64 = existing_transactions
            .iter()
            .filter(|tx| {
                matches!(
                    tx.transaction.status,
                    PaymentStatusEnum::Pending
                        | PaymentStatusEnum::Ready
                        | PaymentStatusEnum::Settled
                )
            })
            .map(|tx| tx.transaction.amount)
            .sum();

        // Prevent payment if invoice is already fully covered
        if active_payment_sum >= invoice.invoice.total {
            return Err(Report::new(StoreError::PaymentError(format!(
                "Invoice already has sufficient payments. Total: {}, Already paid: {}",
                invoice.invoice.total, active_payment_sum
            ))));
        }

        // Prevent payment if this would exceed the invoice total
        let proposed_payment = invoice.invoice.amount_due;
        if active_payment_sum + proposed_payment > invoice.invoice.total {
            return Err(Report::new(StoreError::PaymentError(format!(
                "Payment of {} would exceed invoice total. Already paid: {}, Total: {}",
                proposed_payment, active_payment_sum, invoice.invoice.total
            ))));
        }

        // Create a payment transaction
        let transaction = PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id,
            invoice_id: Some(invoice_id),
            provider_transaction_id: None,
            amount: invoice.invoice.amount_due,
            currency: invoice.invoice.currency.clone(),
            payment_method_id: Some(payment_method_id),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
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
                on_session,
            )
            .await?;

        // Transient next_action (carries the client secret) — surfaced to the
        // on-session caller so the portal can complete 3DS; never persisted.
        let next_action = payment_intent.next_action.clone();

        // Consolidate the transaction
        let tx = self
            .store
            .consolidate_intent_and_transaction_tx(
                conn,
                &Actor::System,
                inserted_transaction.into(),
                payment_intent,
            )
            .await?;

        Ok((tx, next_action))
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_payment_intent(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        payment_method_id: &CustomerPaymentMethodId,
        transaction_id: &PaymentTransactionId,
        amount: u64,
        currency: String,
        on_session: bool,
    ) -> StoreResult<PaymentIntent> {
        let method = CustomerPaymentMethodRow::get_by_id(conn, tenant_id, payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        // Idempotency key derived from the transaction id: same transaction
        // retried after a network blip returns the original outcome from the
        // provider instead of double-charging.
        let request = ChargeRequest {
            transaction_id: *transaction_id,
            customer_external_id: &connection.external_customer_id,
            payment_method_external_id: &method.external_payment_method_id,
            payment_method_type: method.payment_method_type.clone().into(),
            amount_minor: amount as i64,
            currency: &currency,
            idempotency_key: IdempotencyKey::new(format!("charge:{}", transaction_id.as_base62())),
            on_session,
        };

        let outcome = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.charge_off_session(&connector, request),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?
        .change_context_lazy(|| StoreError::PaymentProviderError)?;

        Ok(payment_intent_from_outcome(
            outcome,
            *transaction_id,
            *tenant_id,
            amount as i64,
            currency,
        ))
    }
}
