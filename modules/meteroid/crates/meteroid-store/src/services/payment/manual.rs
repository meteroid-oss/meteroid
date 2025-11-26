use crate::StoreResult;
use crate::domain::Invoice;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::repositories::outbox::OutboxInterface;
use crate::services::Services;
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::enums::{
    InvoicePaymentStatus, InvoiceStatusEnum, PaymentStatusEnum, PaymentTypeEnum,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRowNew;
use error_stack::Report;
use rust_decimal::Decimal;

impl Services {
    /// Adds a manual payment transaction to an invoice.
    /// This is used for recording payments received outside the system (e.g., bank transfers, cash, checks).
    pub async fn add_manual_payment_transaction(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        amount: Decimal,
        payment_date: NaiveDateTime,
        reference: Option<String>,
    ) -> StoreResult<PaymentTransaction> {
        let transaction = self
            .store
            .transaction(|conn| {
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if invoice.invoice.status != InvoiceStatusEnum::Finalized {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Invoice must be in Finalized status to add manual payments"
                                .to_string(),
                        )));
                    }

                    if amount <= Decimal::ZERO {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Payment amount must be positive".to_string(),
                        )));
                    }

                    let currency =
                        rusty_money::iso::find(&invoice.invoice.currency).ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument("Invalid currency".into()))
                        })?;

                    let amount_cents =
                        amount
                            .to_subunit_opt(currency.exponent as u8)
                            .ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(format!(
                                    "Invalid amount for currency {}",
                                    invoice.invoice.currency
                                )))
                            })?;

                    // Validate amount doesn't exceed amount_due
                    if amount_cents > invoice.invoice.amount_due {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Payment amount ({}) exceeds invoice amount due ({})",
                            amount_cents, invoice.invoice.amount_due
                        ))));
                    }

                    let transaction_id = PaymentTransactionId::new();
                    let transaction_new = PaymentTransactionRowNew {
                        id: transaction_id,
                        tenant_id,
                        invoice_id,
                        provider_transaction_id: reference.clone(),
                        amount: amount_cents,
                        currency: invoice.invoice.currency.clone(),
                        payment_method_id: None, // Manual payment has no payment method
                        status: PaymentStatusEnum::Settled,
                        payment_type: PaymentTypeEnum::Payment,
                        error_type: None,
                        processed_at: Some(payment_date),
                    };

                    let inserted_transaction = transaction_new
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let transaction: PaymentTransaction = inserted_transaction.clone().into();
                    self.store
                        .insert_outbox_event_tx(
                            conn,
                            OutboxEvent::payment_transaction_saved(transaction.clone().into()),
                        )
                        .await?;

                    Ok(inserted_transaction.into())
                }
                .scope_boxed()
            })
            .await?;

        Ok(transaction)
    }

    /// Marks an invoice as paid by creating a manual payment transaction for the full amount due.
    /// This validates that the provided amount matches the invoice's amount_due and updates the invoice status.
    pub async fn mark_invoice_as_paid(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        total_amount: Decimal,
        payment_date: NaiveDateTime,
        reference: Option<String>,
    ) -> StoreResult<crate::domain::DetailedInvoice> {
        let invoice = self
            .store
            .transaction(|conn| {
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if invoice.invoice.status != InvoiceStatusEnum::Finalized {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Invoice must be in Finalized status to mark as paid".to_string(),
                        )));
                    }

                    let currency =
                        rusty_money::iso::find(&invoice.invoice.currency).ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument("Invalid currency".into()))
                        })?;

                    let amount_cents = total_amount
                        .to_subunit_opt(currency.exponent as u8)
                        .ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument(format!(
                                "Invalid amount for currency {}",
                                invoice.invoice.currency
                            )))
                        })?;

                    if amount_cents != invoice.invoice.amount_due {
                        return Err(Report::new(StoreError::InvalidArgument(format!(
                            "Payment amount ({}) must match invoice amount due ({})",
                            amount_cents, invoice.invoice.amount_due
                        ))));
                    }

                    let transaction_id = PaymentTransactionId::new();
                    let transaction_new = PaymentTransactionRowNew {
                        id: transaction_id,
                        tenant_id,
                        invoice_id,
                        provider_transaction_id: reference.clone(),
                        amount: amount_cents,
                        currency: invoice.invoice.currency.clone(),
                        payment_method_id: None,
                        status: PaymentStatusEnum::Settled,
                        payment_type: PaymentTypeEnum::Payment,
                        error_type: None,
                        processed_at: Some(payment_date),
                    };

                    let inserted_transaction = transaction_new
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    // Update invoice payment status synchronously
                    InvoiceRow::apply_payment_status(
                        conn,
                        invoice_id,
                        tenant_id,
                        InvoicePaymentStatus::Paid,
                        Some(payment_date),
                    )
                    .await?;

                    let transaction: PaymentTransaction = inserted_transaction.into();
                    self.store
                        .insert_outbox_event_tx(
                            conn,
                            OutboxEvent::payment_transaction_saved(transaction.clone().into()),
                        )
                        .await?;

                    let invoice: Invoice = invoice.invoice.try_into()?;

                    self.store
                        .insert_outbox_event_tx(conn, OutboxEvent::invoice_paid((&invoice).into()))
                        .await?;

                    let updated_invoice = self
                        .store
                        .get_detailed_invoice_by_id_with_conn(conn, tenant_id, invoice_id)
                        .await?;

                    Ok(updated_invoice)
                }
                .scope_boxed()
            })
            .await?;

        Ok(invoice)
    }
}
