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
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;

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
            // We only reach here for a row that carries a provider id (the worker
            // query and the guard above both require one). A bare 404 on an id WE
            // STORED is essentially an account/env mismatch, NOT a lost payment.
            // Cancelling is terminal and would make the skip-if-terminal guard drop
            // a later genuine settlement webhook, so never auto-cancel: log loudly
            // and leave the row Pending for manual review.
            log::error!(
                "Reconciliation: provider has no record of transaction {transaction_id} \
                 (external_id {external_id}, tenant {tenant_id}); NOT cancelling — a stored \
                 provider id with a 404 is almost always an account/env mismatch. Left Pending."
            );
            return Ok(());
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
        // The provider has no record of an id WE STORED. The worker's query only
        // ever selects rows that already carry a provider id, so this is almost
        // always an account/env mismatch, never a lost outbound POST (a row with
        // no provider id is never polled). Do NOT cancel: cancellation is terminal
        // and would drop a later genuine settlement webhook. No state change — the
        // caller logs loudly and leaves the row Pending for manual review.
        RemoteTransactionStatus::Unknown => None,
        RemoteTransactionStatus::Succeeded {
            amount_received_minor,
            processed_at,
            ..
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::payment::model::DeclineKind;
    use common_domain::ids::{BaseId, PaymentTransactionId, TenantId};
    use diesel_models::payments::PaymentTransactionRow;

    fn pending_row_with_provider_id() -> PaymentTransactionRow {
        let now = chrono::Utc::now();
        PaymentTransactionRow {
            id: PaymentTransactionId::new(),
            tenant_id: TenantId::new(),
            invoice_id: None,
            provider_transaction_id: Some("pi_stored_123".to_string()),
            processed_at: None,
            refunded_at: None,
            amount: 3500,
            currency: "EUR".to_string(),
            payment_method_id: None,
            status: diesel_models::enums::PaymentStatusEnum::Pending,
            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
            error_type: None,
            receipt_pdf_id: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            created_at: now,
            next_action: None,
            initiated_by_customer_id: None,
            amount_refunded: 0,
            pending_provider_intent_id: None,
            pending_connection_id: None,
        }
    }

    /// Finding 3: a provider 404 on an id WE STORED must NOT drive the row to a
    /// terminal Cancelled state (which would drop a later genuine settlement
    /// webhook). It yields no state change; the caller logs and leaves it Pending.
    #[test]
    fn unknown_with_provider_id_is_never_cancelled() {
        let row = pending_row_with_provider_id();
        let intent = payment_intent_from_remote_status(
            RemoteTransactionStatus::Unknown,
            row.id,
            row.tenant_id,
            &row,
        );
        assert!(
            intent.is_none(),
            "Unknown must not produce a Cancelled (or any) state change"
        );
    }

    #[test]
    fn pending_is_a_no_op() {
        let row = pending_row_with_provider_id();
        assert!(
            payment_intent_from_remote_status(
                RemoteTransactionStatus::Pending,
                row.id,
                row.tenant_id,
                &row
            )
            .is_none()
        );
    }

    #[test]
    fn succeeded_settles_with_received_amount() {
        let row = pending_row_with_provider_id();
        let processed_at = chrono::Utc::now().naive_utc();
        let intent = payment_intent_from_remote_status(
            RemoteTransactionStatus::Succeeded {
                amount_received_minor: 3500,
                currency: "EUR".into(),
                processed_at,
            },
            row.id,
            row.tenant_id,
            &row,
        )
        .expect("succeeded yields an intent");
        assert_eq!(intent.status, PaymentStatusEnum::Settled);
        assert_eq!(intent.amount_received, Some(3500));
        assert_eq!(intent.processed_at, Some(processed_at));
        assert_eq!(intent.external_id, "pi_stored_123");
    }

    #[test]
    fn cancelled_and_failed_map_through() {
        let row = pending_row_with_provider_id();
        let cancelled = payment_intent_from_remote_status(
            RemoteTransactionStatus::Cancelled,
            row.id,
            row.tenant_id,
            &row,
        )
        .expect("cancelled yields an intent");
        assert_eq!(cancelled.status, PaymentStatusEnum::Cancelled);

        let failed = payment_intent_from_remote_status(
            RemoteTransactionStatus::Failed {
                code: Some("card_declined".to_string()),
                message: "declined".to_string(),
                decline_kind: DeclineKind::Other,
            },
            row.id,
            row.tenant_id,
            &row,
        )
        .expect("failed yields an intent");
        assert_eq!(failed.status, PaymentStatusEnum::Failed);
        assert_eq!(failed.last_payment_error.as_deref(), Some("declined"));
    }
}
