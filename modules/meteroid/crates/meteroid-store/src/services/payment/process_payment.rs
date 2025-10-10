use crate::StoreResult;
use crate::adapters::payment_service_providers::initialize_payment_provider;
use crate::domain::connectors::Connector;
use crate::domain::payment_transactions::{PaymentIntent, PaymentTransaction};
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
use diesel_models::payments::PaymentTransactionRowNew;
use error_stack::{Report, ResultExt};

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
                .attach("Cannot process payment for this invoice status"));
        }

        if invoice.invoice.amount_due <= 0 {
            return Err(Report::new(StoreError::BillingError).attach("Invoice has no amount due"));
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
            .store
            .consolidate_intent_and_transaction_tx(
                conn,
                inserted_transaction.into(),
                payment_intent,
            )
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
}
