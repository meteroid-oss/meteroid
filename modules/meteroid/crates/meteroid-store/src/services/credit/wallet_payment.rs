//! Charging a top-up: create the provider intent unconfirmed, persist its id, then confirm that id.
//! No connection or transaction stays open across provider I/O, and a lost create reply is
//! harmless: nothing was charged.
use crate::adapters::payment::bridge::payment_intent_from_outcome;
use crate::adapters::payment::model::{ChargeOutcome, IdempotencyKey, PrepareWalletChargeRequest};
use crate::adapters::payment::{ConnectorError, PaymentConnector, initialize_payment_connector};
use crate::domain::connectors::Connector;
use crate::domain::payment_transactions::{PaymentIntent, PaymentTransaction};
use crate::domain::{PaymentMethodTypeEnum, PaymentStatusEnum};
use crate::errors::StoreError;
use crate::services::Services;
use crate::{Store, StoreResult};
use common_domain::ids::{BaseId, CustomerId, PaymentTransactionId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;

const PROVIDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

struct PaymentContext {
    customer_id: CustomerId,
    transaction: PaymentTransaction,
    connector: Connector,
    customer_external_id: String,
    method_external_id: String,
    method_type: PaymentMethodTypeEnum,
}

impl Services {
    /// Moves the top-up payment forward from its durable state: prepares and pins the provider
    /// intent when none is pinned yet, then confirms (or, when the purchase was withdrawn,
    /// cancels) that intent.
    pub(crate) async fn resume_prepared_wallet_payment(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        transaction_id: PaymentTransactionId,
        customer_initiated: bool,
    ) -> StoreResult<PaymentIntent> {
        let context = self
            .wallet_payment_context(tenant_id, customer_id, transaction_id)
            .await?;
        let provider = initialize_payment_connector(&context.connector)
            .change_context(StoreError::PaymentProviderError)?;
        self.drive_wallet_payment(context, provider.as_ref(), customer_initiated)
            .await
    }

    async fn wallet_payment_context(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        transaction_id: PaymentTransactionId,
    ) -> StoreResult<PaymentContext> {
        let mut conn = self.store.get_conn().await?;
        let transaction: PaymentTransaction =
            PaymentTransactionRow::get_by_id(&mut conn, transaction_id, tenant_id)
                .await?
                .into();
        let method_id = transaction
            .payment_method_id
            .ok_or_else(|| super::precondition("a top-up payment has no saved payment method"))?;
        let method = CustomerPaymentMethodRow::get_by_id(&mut conn, &tenant_id, &method_id).await?;
        if method.customer_id != customer_id {
            return Err(Report::new(StoreError::ValueNotFound(
                "payment method".to_owned(),
            )));
        }
        let connection =
            CustomerConnectionDetailsRow::get_by_id(&mut conn, &tenant_id, &method.connection_id)
                .await?;
        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;
        Ok(PaymentContext {
            customer_id,
            transaction,
            connector,
            customer_external_id: connection.external_customer_id,
            method_external_id: method.external_payment_method_id,
            method_type: method.payment_method_type.into(),
        })
    }

    async fn drive_wallet_payment(
        &self,
        context: PaymentContext,
        provider: &dyn PaymentConnector,
        customer_initiated: bool,
    ) -> StoreResult<PaymentIntent> {
        let tenant_id = context.transaction.tenant_id;
        let customer_id = context.customer_id;
        let mut transaction = context.transaction;
        if transaction.provider_transaction_id.is_none() {
            if let Some(reason) = self
                .store
                .hooks
                .credit
                .confirmation_cancelled(&self.store, tenant_id, customer_id, &transaction)
                .await?
            {
                return Err(rejected(reason));
            }
            if transaction.status == PaymentStatusEnum::Cancelled {
                // A restart can land between a committed rejection and the purchase bookkeeping.
                return Err(rejected(transaction.error_type.unwrap_or_else(|| {
                    "the unconfirmed payment was cancelled".to_owned()
                })));
            }
            if !matches!(
                transaction.status,
                PaymentStatusEnum::Pending | PaymentStatusEnum::Ready
            ) {
                return Err(super::precondition(
                    "a closed top-up payment cannot be prepared again",
                ));
            }
            let prepared = match context.connector.ensure_active() {
                Err(inactive) => Err(Report::new(ConnectorError::WalletPreparationRejected(
                    inactive.current_context().to_string(),
                ))),
                Ok(()) => tokio::time::timeout(
                    PROVIDER_TIMEOUT,
                    provider.prepare_wallet_charge(PrepareWalletChargeRequest {
                        transaction_id: transaction.id,
                        customer_external_id: &context.customer_external_id,
                        payment_method_external_id: &context.method_external_id,
                        payment_method_type: context.method_type.clone(),
                        amount_minor: transaction.amount,
                        currency: &transaction.currency,
                        idempotency_key: IdempotencyKey::new(format!(
                            "wallet_prepare:v1:{}",
                            transaction.id.as_base62()
                        )),
                    }),
                )
                .await
                .map_err(|_| {
                    Report::new(StoreError::PaymentProviderError)
                        .attach("top-up payment preparation timed out")
                })?,
            };
            self.store
                .note_rejected_credentials(&context.connector, &prepared)
                .await;
            transaction = pin_preparation(&self.store, transaction, prepared).await?;
        }
        if let Some(reason) = self
            .store
            .hooks
            .credit
            .confirmation_cancelled(&self.store, tenant_id, customer_id, &transaction)
            .await?
        {
            transaction.status = PaymentStatusEnum::Cancelled;
            transaction.error_type = Some(reason);
        }
        let outcome = tokio::time::timeout(
            PROVIDER_TIMEOUT,
            provider.resume_wallet_charge(&transaction, customer_initiated),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("top-up payment confirmation remains unresolved")
        })?;
        self.store
            .note_rejected_credentials(&context.connector, &outcome)
            .await;
        let mut outcome = outcome.change_context(StoreError::PaymentProviderError)?;
        if transaction.status == PaymentStatusEnum::Cancelled
            && let ChargeOutcome::Cancelled(cancelled) = &mut outcome
            && let Some(reason) = transaction.error_type.as_ref()
        {
            cancelled.message = reason.clone();
        }
        Ok(payment_intent_from_outcome(
            outcome,
            transaction.id,
            transaction.tenant_id,
            transaction.amount,
            transaction.currency.clone(),
        ))
    }
}

fn rejected(reason: String) -> Report<StoreError> {
    Report::new(ConnectorError::WalletPreparationRejected(reason))
        .change_context(StoreError::PaymentProviderError)
}

/// Commits the prepared intent id before anything is confirmed. A rejected preparation cancels the
/// still-unpinned payment first, so a concurrent preparation can never pin an abandoned intent;
/// an id another worker already pinned wins over this one.
async fn pin_preparation(
    store: &Store,
    transaction: PaymentTransaction,
    prepared: Result<String, Report<ConnectorError>>,
) -> StoreResult<PaymentTransaction> {
    let preparation = match prepared {
        Ok(id) => Ok(id),
        Err(error) => match error.current_context() {
            ConnectorError::WalletPreparationRejected(reason) => Err(reason.clone()),
            ConnectorError::Unsupported { .. } => Err(format!(
                "this payment method's provider does not support top-ups: {}",
                error.current_context()
            )),
            _ => return Err(error.change_context(StoreError::PaymentProviderError)),
        },
    };
    let decision: Result<PaymentTransaction, String> = store
        .transaction(|conn| {
            async move {
                let (_, current) = PaymentTransactionRow::lock_with_customer(
                    conn,
                    transaction.tenant_id,
                    transaction.id,
                )
                .await?;
                if current.amount != transaction.amount
                    || current.currency != transaction.currency
                    || current.payment_method_id != transaction.payment_method_id
                {
                    return Err(super::precondition(
                        "the top-up payment changed during preparation",
                    ));
                }
                if current.provider_transaction_id.is_some() {
                    return Ok(Ok(current.into()));
                }
                if !matches!(
                    current.status,
                    diesel_models::enums::PaymentStatusEnum::Pending
                        | diesel_models::enums::PaymentStatusEnum::Ready
                ) {
                    return Err(super::precondition(
                        "the top-up payment closed before confirmation",
                    ));
                }
                let patch = match &preparation {
                    Ok(id) => PaymentTransactionRowPatch {
                        id: current.id,
                        provider_transaction_id: Some(Some(id.clone())),
                        ..Default::default()
                    },
                    Err(reason) => PaymentTransactionRowPatch {
                        id: current.id,
                        status: Some(diesel_models::enums::PaymentStatusEnum::Cancelled),
                        error_type: Some(Some(reason.clone())),
                        ..Default::default()
                    },
                };
                let updated = patch.patch(conn, current.tenant_id, current.id).await?;
                Ok(match preparation {
                    Ok(_) => Ok(updated.into()),
                    Err(reason) => Err(reason),
                })
            }
            .scope_boxed()
        })
        .await?;
    decision.map_err(rejected)
}

/// The purchase can never succeed with this payment: it was refused before anything was charged.
pub(super) fn preparation_rejected(error: &Report<StoreError>) -> Option<String> {
    match error.downcast_ref::<ConnectorError>() {
        Some(ConnectorError::WalletPreparationRejected(reason)) => Some(reason.clone()),
        _ => None,
    }
}
