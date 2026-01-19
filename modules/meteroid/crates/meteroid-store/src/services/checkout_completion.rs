use crate::StoreResult;
use crate::domain::payment_transactions::{PaymentIntent, PaymentTransaction};
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::{PaymentTransactionRowNew, PaymentTransactionRowPatch};
use error_stack::{Report, ResultExt};

/// Result of charging a payment method directly (before invoice/subscription creation)
#[derive(Debug, Clone)]
pub struct DirectChargeResult {
    pub payment_intent: PaymentIntent,
    pub transaction_id: PaymentTransactionId,
    pub amount: i64,
    pub currency: String,
    pub payment_method_id: CustomerPaymentMethodId,
}

impl Services {
    /// Charges a payment method directly without an existing invoice.
    /// This is used in the self-serve checkout flow to charge the customer
    pub(crate) async fn charge_payment_method_directly(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        payment_method_id: CustomerPaymentMethodId,
        amount: i64,
        currency: String,
    ) -> StoreResult<DirectChargeResult> {
        use crate::adapters::payment_service_providers::initialize_payment_provider;
        use crate::domain::connectors::Connector;

        if amount <= 0 {
            return Err(Report::new(StoreError::InvalidArgument(
                "Amount must be positive".to_string(),
            )));
        }

        let method = CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, &tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;

        let provider = initialize_payment_provider(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let transaction_id = PaymentTransactionId::new();

        let payment_intent = provider
            .create_payment_intent_in_provider(
                &connector,
                &transaction_id,
                &connection.external_customer_id,
                &method.external_payment_method_id,
                &method.payment_method_type.into(),
                amount,
                &currency,
            )
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        match payment_intent.status {
            crate::domain::PaymentStatusEnum::Settled
            | crate::domain::PaymentStatusEnum::Pending => Ok(DirectChargeResult {
                payment_intent,
                transaction_id,
                amount,
                currency,
                payment_method_id,
            }),
            crate::domain::PaymentStatusEnum::Failed => Err(Report::new(StoreError::PaymentError(
                payment_intent
                    .last_payment_error
                    .unwrap_or_else(|| "Payment failed".to_string()),
            ))),
            crate::domain::PaymentStatusEnum::Cancelled => Err(Report::new(
                StoreError::PaymentError("Payment was cancelled".to_string()),
            )),
            crate::domain::PaymentStatusEnum::Ready => {
                // This shouldn't happen for a payment intent, but handle it
                Err(Report::new(StoreError::PaymentError(
                    "Payment intent in unexpected state".to_string(),
                )))
            }
        }
    }

    /// Creates a transaction record for a direct charge result and links it to an invoice.
    pub(crate) async fn create_transaction_for_direct_charge(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        charge_result: &DirectChargeResult,
    ) -> StoreResult<PaymentTransaction> {
        let status: PaymentStatusEnum = charge_result.payment_intent.status.clone().into();

        let transaction = PaymentTransactionRowNew {
            id: charge_result.transaction_id,
            tenant_id,
            invoice_id: Some(invoice_id),
            provider_transaction_id: Some(charge_result.payment_intent.external_id.clone()),
            amount: charge_result.amount,
            currency: charge_result.currency.clone(),
            payment_method_id: Some(charge_result.payment_method_id),
            status,
            payment_type: PaymentTypeEnum::Payment,
            error_type: charge_result.payment_intent.last_payment_error.clone(),
            processed_at: charge_result.payment_intent.processed_at,
            checkout_session_id: None,
        };

        let inserted = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted.into())
    }

    /// Creates a transaction record for a checkout payment (no invoice yet).
    pub(crate) async fn create_transaction_for_checkout(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        charge_result: &DirectChargeResult,
    ) -> StoreResult<PaymentTransaction> {
        let status: PaymentStatusEnum = charge_result.payment_intent.status.clone().into();

        let transaction = PaymentTransactionRowNew {
            id: charge_result.transaction_id,
            tenant_id,
            invoice_id: None,
            provider_transaction_id: Some(charge_result.payment_intent.external_id.clone()),
            amount: charge_result.amount,
            currency: charge_result.currency.clone(),
            payment_method_id: Some(charge_result.payment_method_id),
            status,
            payment_type: PaymentTypeEnum::Payment,
            error_type: charge_result.payment_intent.last_payment_error.clone(),
            processed_at: charge_result.payment_intent.processed_at,
            checkout_session_id: Some(checkout_session_id),
        };

        let inserted = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted.into())
    }

    pub(crate) async fn link_transaction_to_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        transaction_id: PaymentTransactionId,
        invoice_id: InvoiceId,
    ) -> StoreResult<PaymentTransaction> {
        let updated = PaymentTransactionRowPatch {
            invoice_id: Some(Some(invoice_id)),
            ..Default::default()
        }
        .patch(conn, tenant_id, transaction_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(updated.into())
    }
}
