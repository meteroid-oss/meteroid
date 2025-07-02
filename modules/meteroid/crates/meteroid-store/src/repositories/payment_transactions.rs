use crate::store::{PgConn, Store};
use crate::{StoreResult, domain};
use diesel_async::scoped_futures::ScopedFutureExt;

use crate::domain::PaymentIntent;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::PaymentTransaction;
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
    ) -> StoreResult<Vec<PaymentTransaction>>;

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
    ) -> error_stack::Result<PaymentTransaction, StoreError>;

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
    ) -> StoreResult<Vec<PaymentTransaction>> {
        let mut conn = self.get_conn().await?;
        PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|rows| {
                rows.into_iter()
                    .map(|row| row.into())
                    .collect::<Vec<PaymentTransaction>>()
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
            .map(|row_opt| row_opt.map(|row| row.into()))
    }

    async fn consolidate_intent_and_transaction_tx(
        &self,
        conn: &mut PgConn,
        transaction: PaymentTransaction,
        payment_intent: PaymentIntent,
    ) -> error_stack::Result<PaymentTransaction, StoreError> {
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

        // Only update if the status has changed
        if transaction.status == payment_intent.status {
            log::debug!(
                "Transaction {} status unchanged: {:?}",
                transaction.id,
                payment_intent.status
            );
            return Ok(transaction);
        }

        log::info!(
            "Updating transaction {} status from {:?} to {:?}",
            transaction.id,
            transaction.status,
            payment_intent.status
        );

        let patch = PaymentTransactionRowPatch {
            id: transaction.id,
            status: Some(payment_intent.status.clone().into()),
            processed_at: Some(payment_intent.processed_at),
            refunded_at: None,
            error_type: Some(payment_intent.last_payment_error),
        };

        let updated_transaction = self
            .transaction_with(conn, |conn| {
                async move {
                    let updated_transaction = patch.update(conn).await?;

                    let transaction: PaymentTransaction = updated_transaction.into();

                    self.internal
                        .insert_outbox_events_tx(
                            conn,
                            vec![OutboxEvent::payment_transaction_saved(
                                transaction.clone().into(),
                            )],
                        )
                        .await?;

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
            .map(|row| row.into())
    }
}
