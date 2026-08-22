use crate::StoreResult;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
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

/// Facts about a synchronously-declined checkout charge, attached to the returned
/// error so `complete_checkout` can record the failed attempt once its transaction
/// (and the session's `FOR UPDATE` lock) has unwound — see [`Services::persist_declined_checkout_charge`].
#[derive(Debug, Clone)]
pub(crate) struct DeclinedCheckoutCharge {
    pub transaction_id: PaymentTransactionId,
    pub provider_transaction_id: Option<String>,
    pub payment_method_id: CustomerPaymentMethodId,
    pub amount: i64,
    pub currency: String,
    pub error_message: String,
    pub processed_at: Option<chrono::NaiveDateTime>,
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
        use crate::adapters::payment::bridge::payment_intent_from_outcome;
        use crate::adapters::payment::initialize_payment_connector;
        use crate::adapters::payment::model::{ChargeRequest, IdempotencyKey};
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

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let transaction_id = PaymentTransactionId::new();

        let request = ChargeRequest {
            transaction_id,
            customer_external_id: &connection.external_customer_id,
            payment_method_external_id: &method.external_payment_method_id,
            payment_method_type: method.payment_method_type.clone().into(),
            amount_minor: amount,
            currency: &currency,
            idempotency_key: IdempotencyKey::new(format!("charge:{}", transaction_id.as_base62())),
            // Checkout is always customer-present, so 3DS can be completed inline.
            on_session: true,
        };

        let outcome = connector_impl
            .charge_off_session(&connector, request)
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        let payment_intent = payment_intent_from_outcome(
            outcome,
            transaction_id,
            tenant_id,
            amount,
            currency.clone(),
        );

        match payment_intent.status {
            crate::domain::PaymentStatusEnum::Settled
            | crate::domain::PaymentStatusEnum::Pending => Ok(DirectChargeResult {
                payment_intent,
                transaction_id,
                amount,
                currency,
                payment_method_id,
            }),
            crate::domain::PaymentStatusEnum::Failed => {
                let error_message = payment_intent
                    .last_payment_error
                    .clone()
                    .unwrap_or_else(|| "Payment failed".to_string());
                // Decline facts travel on the error; recorded after the tx unwinds.
                Err(
                    Report::new(StoreError::PaymentError(error_message.clone())).attach_opaque(
                        DeclinedCheckoutCharge {
                            transaction_id,
                            provider_transaction_id: Some(payment_intent.external_id.clone()),
                            payment_method_id,
                            amount,
                            currency: currency.clone(),
                            error_message,
                            processed_at: payment_intent.processed_at,
                        },
                    ),
                )
            }
            crate::domain::PaymentStatusEnum::Cancelled => Err(Report::new(
                StoreError::PaymentError("Payment was cancelled".to_string()),
            )),
            crate::domain::PaymentStatusEnum::Ready
            | crate::domain::PaymentStatusEnum::Refunded => {
                // A fresh charge is never Ready or already-Refunded; treat as unexpected.
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
        pending_plan_version_id: Option<common_domain::ids::PlanVersionId>,
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
            pending_plan_version_id,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
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

        // Persist the 3DS/redirect action so a re-completion's idempotency guard
        // (find_active_checkout_transaction) can re-hydrate it. The client_secret
        // is #[serde(skip)]'d, so only the non-secret identity is stored; the
        // portal re-fetches the secret from the provider to resume.
        let next_action = charge_result
            .payment_intent
            .next_action
            .as_ref()
            .and_then(|a| serde_json::to_value(a).ok());

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
            pending_plan_version_id: None,
            next_action,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
        };

        let inserted = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted.into())
    }

    /// Returns the existing non-terminal (Pending/Ready) transaction for a
    /// checkout session, if any, alongside its persisted next_action. Backs the
    /// checkout-completion idempotency guard: a re-completion while a charge is
    /// in flight returns this instead of issuing a second charge.
    ///
    /// The transient `client_secret` on `UseSdk` is `#[serde(skip)]`'d, so a
    /// re-fetched next_action carries none — the portal re-fetches it from the
    /// provider to resume 3DS. The domain conversion ghosts next_action to None,
    /// so it is re-hydrated from the row's JSONB here.
    pub(crate) async fn find_active_checkout_transaction(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
    ) -> StoreResult<Option<(PaymentTransaction, Option<PaymentNextAction>)>> {
        let Some(row) =
            diesel_models::payments::PaymentTransactionRow::get_active_by_checkout_session_id(
                conn,
                checkout_session_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };

        let next_action: Option<PaymentNextAction> = row
            .next_action
            .clone()
            .and_then(|v| serde_json::from_value(v).ok());

        let mut transaction: PaymentTransaction = row.into();
        transaction.next_action = next_action.clone();

        Ok(Some((transaction, next_action)))
    }

    /// Persist a declined checkout charge (from the error's [`DeclinedCheckoutCharge`]
    /// attachment) as a committed `Failed` row. Call ONLY after the checkout tx has
    /// returned (session `FOR UPDATE` released), else the FK insert deadlocks.
    pub(crate) async fn persist_declined_checkout_charge(
        &self,
        tenant_id: TenantId,
        checkout_session_id: CheckoutSessionId,
        error: &Report<StoreError>,
    ) {
        let Some(declined) = error
            .frames()
            .find_map(|f| f.downcast_ref::<DeclinedCheckoutCharge>())
        else {
            return;
        };

        let row = PaymentTransactionRowNew {
            id: declined.transaction_id,
            tenant_id,
            invoice_id: None,
            provider_transaction_id: declined.provider_transaction_id.clone(),
            amount: declined.amount,
            currency: declined.currency.clone(),
            payment_method_id: Some(declined.payment_method_id),
            status: PaymentStatusEnum::Failed,
            payment_type: PaymentTypeEnum::Payment,
            error_type: Some(declined.error_message.clone()),
            processed_at: declined.processed_at,
            checkout_session_id: Some(checkout_session_id),
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
        };

        match self.store.get_conn().await {
            Ok(mut conn) => {
                if let Err(e) = row.insert(&mut conn).await {
                    log::error!(
                        "Failed to persist declined checkout charge {}: {e:?}",
                        declined.transaction_id
                    );
                }
            }
            Err(e) => log::error!(
                "Could not acquire a connection to persist declined checkout charge {}: {e:?}",
                declined.transaction_id
            ),
        }
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
