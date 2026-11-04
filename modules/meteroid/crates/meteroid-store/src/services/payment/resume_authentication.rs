//! Resuming a charge left awaiting customer authentication (3DS / SCA).
//!
//! An off-session charge (a renewal) cannot show the customer a challenge: the transaction stays
//! `Pending` with a stored `next_action`, which holds no secret. When the customer comes back to
//! the invoice, the action is re-read from the provider, secret included, and handed to the portal
//! for this one response. Settlement then arrives through the usual webhook / reconciliation path.
//!
//! Handing out the secret is what lets the money move, so the invoice and the connection are
//! re-checked like on any path that starts a payment.
//!
//! TODO(dunning): nothing tells the customer to come back yet. The email asking them to
//! authenticate (sent when an off-session charge enters this state, from both the charge response
//! and the `requires_action` webhook, idempotently) lands with the dunning and email customization
//! work, along with the expiry policy for a charge nobody authenticates.

use crate::StoreResult;
use crate::adapters::payment::bridge::next_action_from_instruction;
use crate::adapters::payment::error::CustomerFacingMessage;
use crate::adapters::payment::initialize_payment_connector;
use crate::domain::connectors::Connector;
use crate::domain::payment_transactions::{PaymentNextAction, row_awaits_authentication};
use crate::errors::StoreError;
use crate::services::Services;
use common_domain::ids::{InvoiceId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::enums::{InvoiceStatusEnum, PaymentStatusEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};
use error_stack::{Report, ResultExt};

/// Refusals are shown to the customer as is.
fn refuse(message: &str) -> Report<StoreError> {
    Report::new(StoreError::FailedPrecondition(message.to_string()))
        .attach_opaque(CustomerFacingMessage(message.to_string()))
}

impl Services {
    pub async fn resume_invoice_payment_authentication(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<PaymentNextAction> {
        let mut conn = self.store.get_conn().await?;

        // Latest first: only one pending attempt may exist per invoice, but never depend on it.
        let awaiting = PaymentTransactionRow::list_by_invoice_id(&mut conn, invoice_id, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?
            .into_iter()
            .filter(|row| row_awaits_authentication(&row.transaction))
            .max_by_key(|row| row.transaction.created_at)
            .ok_or_else(|| refuse("No payment is awaiting authentication on this invoice."))?;
        let transaction = awaiting.transaction;

        let (Some(external_id), Some(method)) =
            (transaction.provider_transaction_id.clone(), awaiting.method)
        else {
            return Err(refuse("This payment cannot be resumed."));
        };

        // The invoice may have been paid another way, voided or credited since the charge.
        let invoice = InvoiceRow::find_by_id(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;
        let payable = invoice.consolidated_into_invoice_id.is_none()
            && matches!(
                invoice.status,
                InvoiceStatusEnum::Draft | InvoiceStatusEnum::Finalized
            )
            && invoice.amount_due > 0
            && transaction.amount <= invoice.amount_due;
        if !payable {
            return Err(refuse(
                "This payment no longer matches what is due on this invoice. Please contact the merchant.",
            ));
        }

        let connection =
            CustomerConnectionDetailsRow::get_by_id(&mut conn, &tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;
        connector.ensure_active()?;
        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let instruction = connector_impl
            .fetch_pending_action(&external_id)
            .await
            .change_context(StoreError::PaymentProviderError)?;

        if let Some(instruction) = instruction {
            return Ok(next_action_from_instruction(instruction).1);
        }

        // Answered or lapsed meanwhile: settle the row from the provider's state rather than wait
        // for the sweep, so the customer's next load shows the outcome.
        if let Err(err) = self
            .reconcile_pending_transaction(transaction.id, tenant_id)
            .await
        {
            log::warn!(
                "Could not reconcile transaction {} after a lapsed authentication: {err:?}",
                transaction.id
            );
        }

        // Still in flight at the provider (authenticated, settling): it no longer waits on the
        // customer, so stop offering the challenge.
        let row = PaymentTransactionRow::get_by_id(&mut conn, transaction.id, tenant_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;
        if row.status == PaymentStatusEnum::Pending && row.next_action.is_some() {
            PaymentTransactionRowPatch {
                id: transaction.id,
                next_action: Some(None),
                ..Default::default()
            }
            .patch(&mut conn, tenant_id, transaction.id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;
        }

        Err(refuse(
            "This payment is no longer awaiting authentication. Please refresh the page.",
        ))
    }
}
