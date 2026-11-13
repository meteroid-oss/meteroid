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
//! `claim_pending_for_reconcile` (Pending rows older than an age threshold
//! that already carry a provider id, each backed off by its age) and calling
//! [`Services::reconcile_pending_transaction`] for each.
//!
//! Refunds. A second pass does the same for REFUND rows still Pending at the provider,
//! through [`Services::reconcile_pending_refund`]. Providers with no refund GET answer
//! `Unsupported`; those rows are skipped and wait for their webhook.

use crate::StoreResult;
use crate::adapters::payment::error::ConnectorError;
use crate::adapters::payment::model::{RemoteRefundStatus, RemoteTransactionStatus};
use crate::domain::PaymentStatusEnum;
use crate::domain::entity_activity::Actor;
use crate::domain::payment_transactions::{PaymentIntent, PaymentTransaction};
use crate::errors::StoreError;
use crate::repositories::payment_transactions::{
    PaymentTransactionInterface, RefundSettlement, RefundSettlementStatus,
};
use crate::services::Services;
use crate::store::PgConn;
use common_domain::ids::{PaymentTransactionId, TenantId};
use diesel_models::payments::PaymentTransactionRow;
use error_stack::ResultExt;
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

        let connector_impl = self.resolve_transaction_connector(&mut conn, &row).await?;

        let remote_status = connector_impl
            .fetch_transaction_status(&external_id)
            .await
            .change_context(StoreError::PaymentProviderError)?;

        // Processing: the customer finished whatever was asked of them (3DS, a redirect).
        if matches!(remote_status, RemoteTransactionStatus::Processing) {
            PaymentTransactionRow::clear_awaited_action(&mut conn, transaction_id, tenant_id)
                .await
                .map_err(Into::<error_stack::Report<StoreError>>::into)?;
        }

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

    /// Resolve a REFUND row still Pending at the provider and apply what it says. Idempotent:
    /// `settle_refund_tx` no-ops on a row that already reached a terminal state. A provider
    /// with no refund GET answers `Unsupported`; the row is left for its webhook.
    pub async fn reconcile_pending_refund(
        &self,
        refund_id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;

        let row = PaymentTransactionRow::get_by_id(&mut conn, refund_id, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        if row.status != diesel_models::enums::PaymentStatusEnum::Pending {
            log::debug!(
                "Refund {} is no longer pending ({:?}); nothing to reconcile",
                refund_id,
                row.status
            );
            return Ok(());
        }

        let Some(external_refund_id) = row.provider_transaction_id.clone() else {
            log::warn!("Refund {refund_id} has no provider refund id; cannot reconcile by id");
            return Ok(());
        };

        let connector_impl = self.resolve_transaction_connector(&mut conn, &row).await?;

        let parent_external_id = match row.parent_transaction_id {
            Some(parent_id) => {
                PaymentTransactionRow::get_by_id(&mut conn, parent_id, tenant_id)
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?
                    .provider_transaction_id
            }
            None => None,
        };
        drop(conn);

        let snapshot = match connector_impl
            .fetch_refund(&external_refund_id, parent_external_id.as_deref())
            .await
        {
            Ok(snapshot) => snapshot,
            Err(err) if matches!(err.current_context(), ConnectorError::Unsupported { .. }) => {
                log::debug!(
                    "Provider cannot resolve refund {external_refund_id}; leaving refund {refund_id} pending"
                );
                return Ok(());
            }
            Err(err) => return Err(err.change_context(StoreError::PaymentProviderError)),
        };

        let status = match snapshot.status {
            // Still in flight at the provider: nothing to record.
            RemoteRefundStatus::Pending => return Ok(()),
            RemoteRefundStatus::Succeeded => RefundSettlementStatus::Settled {
                processed_at: common_domain::clock::now().naive_utc(),
                external_refund_id,
            },
            RemoteRefundStatus::Failed { message, .. } => RefundSettlementStatus::Failed {
                error: message,
                external_refund_id: Some(external_refund_id),
            },
            RemoteRefundStatus::Canceled => RefundSettlementStatus::Failed {
                error: "Cancelled by the payment provider".to_string(),
                external_refund_id: Some(external_refund_id),
            },
        };

        let store = self.store.clone();
        self.store
            .transaction(|conn| {
                let store = store.clone();
                let status = status.clone();
                // Unlocked: `settle_refund_tx` locks the parent first, then re-reads this row.
                let refund: PaymentTransaction = row.clone().into();
                async move {
                    store
                        .settle_refund_tx(conn, &Actor::System, refund, RefundSettlement { status })
                        .await?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        Ok(())
    }

    async fn resolve_transaction_connector(
        &self,
        conn: &mut PgConn,
        row: &PaymentTransactionRow,
    ) -> StoreResult<Box<dyn crate::adapters::payment::PaymentConnector>> {
        super::resolve_connector_for_transaction(&self.store, conn, row)
            .await
            .map(|(_, connector_impl)| connector_impl)
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
        RemoteTransactionStatus::Pending | RemoteTransactionStatus::Processing => None,
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
            decline_kind: None,
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
            decline_kind: None,
            processed_at: None,
        }),
        RemoteTransactionStatus::Failed {
            message,
            decline_kind,
            ..
        } => Some(PaymentIntent {
            external_id: row.provider_transaction_id.clone().unwrap_or_default(),
            transaction_id,
            tenant_id,
            amount_requested: row.amount,
            amount_received: None,
            currency: row.currency.clone(),
            next_action: None,
            status: PaymentStatusEnum::Failed,
            last_payment_error: Some(message),
            decline_kind: Some(decline_kind),
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
        let now = common_domain::clock::now();
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
            store_committed_at: now.naive_utc(),
            created_at: now,
            next_action: None,
            initiated_by_customer_id: None,
            amount_refunded: 0,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            amount_reversed: 0,
            amount_disputed: 0,
            amount_reversed_excess: 0,
            refund_excess: 0,
            reversal_kind: None,
            reversal_reason: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            last_reconciled_at: None,
            decline_kind: None,
            charge_attempt: None,
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
        let processed_at = common_domain::clock::now().naive_utc();
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
