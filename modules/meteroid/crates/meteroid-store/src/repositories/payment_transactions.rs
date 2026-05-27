use crate::store::{PgConn, Store};
use crate::{StoreResult, domain};
use scoped_futures::ScopedFutureExt;

use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::{PaymentIntent, PaymentTransactionWithMethod};
use crate::errors::StoreError;
use common_domain::ids::{InvoiceId, PaymentTransactionId, TenantId};
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};
use error_stack::Report;

#[async_trait::async_trait]
pub trait PaymentTransactionInterface {
    async fn list_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<PaymentTransactionWithMethod>>;

    async fn last_settled_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Option<PaymentTransaction>>;

    async fn consolidate_intent_and_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        payment_intent: PaymentIntent,
    ) -> Result<PaymentTransaction, Report<StoreError>>;

    async fn get_payment_tx_by_id_for_update(
        &self,
        conn: &mut PgConn,
        id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<PaymentTransaction>;
}

#[async_trait::async_trait]
impl PaymentTransactionInterface for Store {
    async fn list_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Vec<PaymentTransactionWithMethod>> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|rows| {
                rows.into_iter()
                    .map(std::convert::Into::into)
                    .collect::<Vec<PaymentTransactionWithMethod>>()
            })
    }

    async fn last_settled_payment_tx_by_invoice_id(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<Option<PaymentTransaction>> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::last_settled_by_invoice_id(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|row_opt| row_opt.map(std::convert::Into::into))
    }

    async fn consolidate_intent_and_transaction_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        transaction: PaymentTransaction,
        payment_intent: PaymentIntent,
    ) -> Result<PaymentTransaction, Report<StoreError>> {
        // Skip processing if the transaction is already in a terminal state
        if transaction.status != domain::enums::PaymentStatusEnum::Pending
            && transaction.status != domain::enums::PaymentStatusEnum::Ready
        {
            log::info!(
                "Transaction {} already in non-pending state: {:?}",
                transaction.id,
                transaction.status
            );
            return Ok(transaction);
        }

        let status_changed = transaction.status != payment_intent.status;

        // An async charge (GoCardless DD, Stripe ACH) returns Pending, so the id
        // arrives with no status change; still persist it once, or reconciliation
        // (filters provider_transaction_id IS NOT NULL) can never recover it.
        let backfill_external_id =
            transaction.provider_transaction_id.is_none() && !payment_intent.external_id.is_empty();

        // Set the 3DS/SCA action when the charge needs one (status stays
        // Pending); clear it once the charge reaches a terminal state.
        let next_action_patch: Option<Option<serde_json::Value>> = match &payment_intent.next_action
        {
            // The client secret is #[serde(skip)] on PaymentNextAction, so it is
            // never written here regardless.
            Some(action) => Some(serde_json::to_value(action).ok()),
            None if status_changed
                && payment_intent.status != domain::enums::PaymentStatusEnum::Pending =>
            {
                Some(None)
            }
            None => None,
        };

        if !status_changed && !backfill_external_id && next_action_patch.is_none() {
            return Ok(transaction);
        }

        let patch = PaymentTransactionRowPatch {
            id: transaction.id,
            invoice_id: None,
            status: status_changed.then(|| payment_intent.status.clone().into()),
            processed_at: status_changed.then_some(payment_intent.processed_at),
            refunded_at: None,
            error_type: status_changed.then_some(payment_intent.last_payment_error),
            provider_transaction_id: backfill_external_id
                .then(|| Some(payment_intent.external_id.clone())),
            next_action: next_action_patch,
        };

        let tenant_id = transaction.tenant_id;
        let updated_transaction = self
            .transaction_with(conn, |conn| {
                async move {
                    let updated_transaction = patch.update(conn).await?;

                    let transaction: PaymentTransaction = updated_transaction.into();

                    // A pure id backfill is not a settlement event; don't broadcast it.
                    if status_changed {
                        self.internal
                            .record_outbox_batch_tx(
                                conn,
                                tenant_id,
                                actor,
                                vec![OutboxEvent::payment_transaction_saved(
                                    transaction.clone().into(),
                                )],
                            )
                            .await?;
                    }

                    // Payment method is resolved dynamically from the customer at billing time
                    // No need to update the subscription's payment method field

                    Ok(transaction)
                }
                .scope_boxed()
            })
            .await?;

        Ok(updated_transaction)
    }

    async fn get_payment_tx_by_id_for_update(
        &self,
        conn: &mut PgConn,
        id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<PaymentTransaction> {
        PaymentTransactionRow::get_by_id_for_update(conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(std::convert::Into::into)
    }
}
