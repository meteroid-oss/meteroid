//! Reconciliation: pull authoritative state from the provider when a webhook
//! never arrived (or arrived late).
//!
//! Why this exists. Stripe / GoCardless / Adyen all retry webhook delivery on
//! a backoff but with no absolute guarantee. If a `payment_intent.succeeded`
//! webhook is lost (network blip on our side, an outage during the retry
//! window, …) the local `PaymentTransaction` stays `Pending` indefinitely.
//! That's an unpaid invoice in our system that the customer actually paid.
//!
//! How it works. The worker periodically calls
//! [`Services::reconcile_pending_transaction`] for each
//! [`PaymentStatusEnum::Pending`] row that has a `provider_transaction_id`.
//! We ask the connector for the authoritative status and feed the result
//! through the same consolidation pipeline webhooks use, so the state
//! machine guarantees (skip-if-terminal, etc.) apply uniformly.
//!
//! Scheduling. The `reconciliation_worker` sweeps on an interval, listing
//! `list_pending_with_provider_id` (Pending rows older than an age threshold
//! that already carry a provider id) and calling
//! [`Services::reconcile_pending_transaction`] for each.

use crate::StoreResult;
use crate::adapters::payment::initialize_payment_connector;
use crate::adapters::payment::model::RemoteTransactionStatus;
use crate::domain::PaymentStatusEnum;
use crate::domain::connectors::Connector;
use crate::domain::payment_transactions::PaymentIntent;
use crate::errors::StoreError;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::Services;
use common_domain::ids::{PaymentTransactionId, TenantId};
use scoped_futures::ScopedFutureExt;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::{Report, ResultExt};

/// Min age before a provider "no record" (`Unknown`) is allowed to cancel a
/// transaction. Cancelling is irreversible (a later settlement webhook is
/// dropped on the terminal row), so we wait out transient 404s.
const UNKNOWN_CANCEL_GRACE: chrono::Duration = chrono::Duration::hours(12);

impl Services {
    /// Reconcile a single pending transaction against the provider. Idempotent:
    /// the underlying consolidation function already skips terminal-state rows.
    /// Returns `Ok(())` whether the provider knew about the transaction or
    /// not — see the `RemoteTransactionStatus::Unknown` branch.
    pub async fn reconcile_pending_transaction(
        &self,
        transaction_id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;

        let row = PaymentTransactionRow::get_by_id(&mut conn, transaction_id, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        if row.status != diesel_models::enums::PaymentStatusEnum::Pending {
            log::debug!(
                "Transaction {} is no longer pending ({:?}); nothing to reconcile",
                transaction_id,
                row.status
            );
            return Ok(());
        }

        let Some(external_id) = row.provider_transaction_id.clone() else {
            log::warn!(
                "Transaction {} has no provider_transaction_id; cannot reconcile by id",
                transaction_id
            );
            return Ok(());
        };

        // Resolve the connector via the payment method → connection chain.
        let method_id = row.payment_method_id.ok_or_else(|| {
            Report::new(StoreError::InvalidArgument(
                "transaction has no payment method".into(),
            ))
        })?;
        let method = CustomerPaymentMethodRow::get_by_id(&mut conn, &tenant_id, &method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;
        let connection =
            diesel_models::customer_connection::CustomerConnectionDetailsRow::get_by_id(
                &mut conn,
                &tenant_id,
                &method.connection_id,
            )
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;
        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let remote_status = connector_impl
            .fetch_transaction_status(&connector, &external_id)
            .await
            .change_context(StoreError::PaymentProviderError)?;

        if matches!(remote_status, RemoteTransactionStatus::Unknown) {
            let age = chrono::Utc::now().naive_utc() - row.created_at;
            if age < UNKNOWN_CANCEL_GRACE {
                log::warn!(
                    "Provider has no record of transaction {transaction_id} ({external_id}) yet \
                     (age {age}); deferring cancellation"
                );
                return Ok(());
            }
            log::warn!(
                "Provider has no record of transaction {transaction_id} ({external_id}) after \
                 {age}; cancelling"
            );
        }

        let Some(intent) =
            payment_intent_from_remote_status(remote_status, transaction_id, tenant_id, &row)
        else {
            // RemoteTransactionStatus::Pending or Unknown — no state change.
            log::debug!(
                "Reconciliation for transaction {} found no state change",
                transaction_id
            );
            return Ok(());
        };

        let store = self.store.clone();
        self.store
            .transaction(|conn| {
                let store = store.clone();
                let intent = intent.clone();
                async move {
                    let existing = store
                        .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                        .await?;
                    store
                        .consolidate_intent_and_transaction_tx(
                            conn,
                            &crate::domain::entity_activity::Actor::System,
                            existing,
                            intent,
                        )
                        .await?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        Ok(())
    }
}

/// Translate the reconciliation result into the internal [`PaymentIntent`] shape
/// the settlement function consumes. Returns `None` if the provider says the
/// transaction is still in-flight (we don't change local state in that case).
fn payment_intent_from_remote_status(
    status: RemoteTransactionStatus,
    transaction_id: PaymentTransactionId,
    tenant_id: TenantId,
    row: &PaymentTransactionRow,
) -> Option<PaymentIntent> {
    match status {
        RemoteTransactionStatus::Pending => None,
        // The provider has no record. The local transaction is presumed lost
        // (our outbound POST never reached them). Cancel it so the invoice
        // can be paid via a fresh attempt.
        RemoteTransactionStatus::Unknown => Some(PaymentIntent {
            external_id: row.provider_transaction_id.clone().unwrap_or_default(),
            transaction_id,
            tenant_id,
            amount_requested: row.amount,
            amount_received: None,
            currency: row.currency.clone(),
            next_action: None,
            status: PaymentStatusEnum::Cancelled,
            last_payment_error: Some("provider has no record of transaction".into()),
            processed_at: None,
        }),
        RemoteTransactionStatus::Succeeded {
            amount_received_minor,
            processed_at,
        } => Some(PaymentIntent {
            external_id: row.provider_transaction_id.clone().unwrap_or_default(),
            transaction_id,
            tenant_id,
            amount_requested: row.amount,
            amount_received: Some(amount_received_minor),
            currency: row.currency.clone(),
            next_action: None,
            status: PaymentStatusEnum::Settled,
            last_payment_error: None,
            processed_at: Some(processed_at),
        }),
        RemoteTransactionStatus::Cancelled => Some(PaymentIntent {
            external_id: row.provider_transaction_id.clone().unwrap_or_default(),
            transaction_id,
            tenant_id,
            amount_requested: row.amount,
            amount_received: None,
            currency: row.currency.clone(),
            next_action: None,
            status: PaymentStatusEnum::Cancelled,
            last_payment_error: None,
            processed_at: None,
        }),
        RemoteTransactionStatus::Failed { message, .. } => Some(PaymentIntent {
            external_id: row.provider_transaction_id.clone().unwrap_or_default(),
            transaction_id,
            tenant_id,
            amount_requested: row.amount,
            amount_received: None,
            currency: row.currency.clone(),
            next_action: None,
            status: PaymentStatusEnum::Failed,
            last_payment_error: Some(message),
            processed_at: None,
        }),
    }
}
