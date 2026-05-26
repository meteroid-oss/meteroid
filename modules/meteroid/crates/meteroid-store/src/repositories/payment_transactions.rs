use crate::store::{PgConn, Store};
use crate::{StoreResult, domain};
use diesel_async::scoped_futures::ScopedFutureExt;

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

        // Persist the provider-side id the first time we learn it, regardless of
        // whether the status moved. The off-session charge path inserts the row
        // with a NULL provider_transaction_id (the id only comes back from the
        // provider *after* the row exists), and an asynchronously-settled charge
        // (GoCardless direct debit, Stripe ACH) comes back Pending — i.e. status
        // unchanged. Without backfilling here, that id would never be stored and
        // the reconciliation worker (which filters `provider_transaction_id IS
        // NOT NULL`) could never recover the transaction if the confirming
        // webhook were lost.
        let backfill_external_id =
            transaction.provider_transaction_id.is_none() && !payment_intent.external_id.is_empty();

        if !status_changed && !backfill_external_id {
            log::debug!(
                "Transaction {} status unchanged ({:?}) and provider id already set; nothing to do",
                transaction.id,
                payment_intent.status
            );
            return Ok(transaction);
        }

        if status_changed {
            log::info!(
                "Updating transaction {} status from {:?} to {:?}",
                transaction.id,
                transaction.status,
                payment_intent.status
            );
        } else {
            log::info!(
                "Backfilling provider_transaction_id for transaction {} (status unchanged: {:?})",
                transaction.id,
                transaction.status
            );
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
        };

        let updated_transaction = self
            .transaction_with(conn, |conn| {
                async move {
                    let updated_transaction = patch.update(conn).await?;

                    let transaction: PaymentTransaction = updated_transaction.into();

                    // Only broadcast a state transition when the status actually
                    // moved — a pure provider-id backfill is not a settlement
                    // event and must not trigger downstream activation.
                    if status_changed {
                        self.internal
                            .insert_outbox_events_tx(
                                conn,
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
