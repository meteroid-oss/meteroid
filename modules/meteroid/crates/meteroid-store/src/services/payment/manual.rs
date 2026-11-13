use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use crate::domain::payment_transactions::PaymentTransaction;
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::services::{InvoiceDelivery, Services};
use chrono::{DateTime, FixedOffset, Utc};
use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_models::enums::{InvoiceStatusEnum, PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

impl Services {
    /// Adds a manual payment transaction to an invoice.
    /// This is used for recording payments received outside the system (e.g., bank transfers, cash, checks).
    pub async fn add_manual_payment_transaction(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        amount: Decimal,
        payment_date: DateTime<FixedOffset>,
        reference: Option<String>,
    ) -> StoreResult<PaymentTransaction> {
        let transaction = self
            .store
            .transaction(|conn| {
                let actor = &actor;
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

                    // A consolidated child is billed via its parent; payments belong on the parent.
                    if invoice.invoice.consolidated_into_invoice_id.is_some() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Cannot add a payment to an invoice merged into a consolidated parent"
                                .to_string(),
                        )));
                    }

                    if amount <= Decimal::ZERO {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Payment amount must be positive".to_string(),
                        )));
                    }
                    check_payment_date(payment_date, common_domain::clock::now())?;

                    let currency = meteroid_money::iso::find(&invoice.invoice.currency)
                        .ok_or_else(|| {
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
                        invoice_id: Some(invoice_id),
                        provider_transaction_id: reference.clone(),
                        amount: amount_cents,
                        currency: invoice.invoice.currency.clone(),
                        payment_method_id: None, // Manual payment has no payment method
                        status: PaymentStatusEnum::Settled,
                        payment_type: PaymentTypeEnum::Payment,
                        error_type: None,
                        processed_at: Some(payment_date.naive_utc()),
                        checkout_session_id: None,
                        pending_plan_version_id: None,
                        next_action: None,
                        initiated_by_customer_id: None,
                        pending_provider_intent_id: None,
                        pending_connection_id: None,
                        parent_transaction_id: None,
                        credit_note_id: None,
                        refund_mode: None,
                        reversal_reason: None,
                    };

                    let inserted_transaction = transaction_new
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let transaction: PaymentTransaction = inserted_transaction.clone().into();
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::payment_transaction_saved(
                                transaction.clone().into(),
                            )],
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
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        total_amount: Decimal,
        payment_date: DateTime<FixedOffset>,
        reference: Option<String>,
    ) -> StoreResult<crate::domain::DetailedInvoice> {
        let invoice = self
            .store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    if invoice.invoice.status != InvoiceStatusEnum::Finalized {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Invoice must be in Finalized status to mark as paid".to_string(),
                        )));
                    }

                    // A consolidated child is billed via its parent; mark the parent paid instead.
                    if invoice.invoice.consolidated_into_invoice_id.is_some() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Cannot mark an invoice merged into a consolidated parent as paid"
                                .to_string(),
                        )));
                    }

                    check_payment_date(payment_date, common_domain::clock::now())?;

                    let currency = meteroid_money::iso::find(&invoice.invoice.currency)
                        .ok_or_else(|| {
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
                        invoice_id: Some(invoice_id),
                        provider_transaction_id: reference.clone(),
                        amount: amount_cents,
                        currency: invoice.invoice.currency.clone(),
                        payment_method_id: None,
                        status: PaymentStatusEnum::Settled,
                        payment_type: PaymentTypeEnum::Payment,
                        error_type: None,
                        processed_at: Some(payment_date.naive_utc()),
                        checkout_session_id: None,
                        pending_plan_version_id: None,
                        next_action: None,
                        initiated_by_customer_id: None,
                        pending_provider_intent_id: None,
                        pending_connection_id: None,
                        parent_transaction_id: None,
                        credit_note_id: None,
                        refund_mode: None,
                        reversal_reason: None,
                    };

                    let inserted_transaction = transaction_new
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let transaction: PaymentTransaction = inserted_transaction.into();
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::payment_transaction_saved(transaction.into())],
                        )
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

/// A payment recorded by hand is dated today at the latest, today where it was recorded (the
/// offset it carries); any past day is accepted, a payment received before the invoice was issued
/// included.
fn check_payment_date(payment_date: DateTime<FixedOffset>, now: DateTime<Utc>) -> StoreResult<()> {
    let offset = payment_date.offset().local_minus_utc();
    if !(LOWEST_UTC_OFFSET_SECS..=HIGHEST_UTC_OFFSET_SECS).contains(&offset) {
        return Err(Report::new(StoreError::InvalidArgument(
            "The payment date has no valid time zone offset".to_string(),
        )));
    }
    if payment_date.date_naive() > now.with_timezone(payment_date.offset()).date_naive() {
        return Err(Report::new(StoreError::InvalidArgument(
            "The payment date cannot be later than today".to_string(),
        )));
    }
    Ok(())
}

/// UTC−12:00 and UTC+14:00, the time zones furthest behind and ahead.
const LOWEST_UTC_OFFSET_SECS: i32 = -12 * 3600;
const HIGHEST_UTC_OFFSET_SECS: i32 = 14 * 3600;

impl Services {
    /// Records a payment of the whole amount due received at `paid_at`, and settles it in the same
    /// transaction (the invoice turns Paid, the credit it sold is funded as of `paid_at`) instead
    /// of leaving that to the settlement consumer, which then finds nothing left to do. `paid_at`
    /// is also the present the funded balance is judged at: this is a replay's entry point.
    pub(crate) async fn settle_invoice_payment_at(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        paid_at: DateTime<Utc>,
        reference: Option<String>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<crate::domain::DetailedInvoice> {
        if paid_at > common_domain::clock::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a payment cannot be received at a future moment".to_string(),
            )));
        }
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .invoice;
                    if invoice.status != InvoiceStatusEnum::Finalized {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Invoice must be in Finalized status to mark as paid".to_string(),
                        )));
                    }
                    if invoice.consolidated_into_invoice_id.is_some() {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Cannot mark an invoice merged into a consolidated parent as paid"
                                .to_string(),
                        )));
                    }
                    if invoice.amount_due <= 0 {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "The invoice has nothing left to pay".to_string(),
                        )));
                    }
                    check_payment_date(paid_at.fixed_offset(), common_domain::clock::now())?;
                    if PaymentTransactionRow::exists_live_for_invoice(conn, invoice_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                    {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "A payment of this invoice is already in progress".to_string(),
                        )));
                    }
                    delivery.apply_tx(conn, tenant_id, invoice_id).await?;
                    let transaction: PaymentTransaction = PaymentTransactionRowNew {
                        id: PaymentTransactionId::new(),
                        tenant_id,
                        invoice_id: Some(invoice_id),
                        provider_transaction_id: reference.clone(),
                        amount: invoice.amount_due,
                        currency: invoice.currency.clone(),
                        payment_method_id: None,
                        status: PaymentStatusEnum::Settled,
                        payment_type: PaymentTypeEnum::Payment,
                        error_type: None,
                        processed_at: Some(paid_at.naive_utc()),
                        checkout_session_id: None,
                        pending_plan_version_id: None,
                        next_action: None,
                        initiated_by_customer_id: None,
                        pending_provider_intent_id: None,
                        pending_connection_id: None,
                        parent_transaction_id: None,
                        credit_note_id: None,
                        refund_mode: None,
                        reversal_reason: None,
                    }
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into();
                    let event: PaymentTransactionEvent = transaction.into();
                    // Still announced, for webhooks and the consumer, whose replay is a no-op.
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::payment_transaction_saved(event.clone())],
                        )
                        .await?;
                    self.on_invoice_payment_settled_tx(conn, &event, invoice_id, paid_at)
                        .await?;
                    self.store
                        .get_detailed_invoice_by_id_with_conn(conn, tenant_id, invoice_id)
                        .await
                }
                .scope_boxed()
            })
            .await
    }
}

impl crate::services::ServicesEdge {
    pub async fn settle_invoice_payment_at(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        paid_at: DateTime<Utc>,
        reference: Option<String>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<crate::domain::DetailedInvoice> {
        self.services
            .settle_invoice_payment_at(actor, tenant_id, invoice_id, paid_at, reference, delivery)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::check_payment_date;
    use chrono::{DateTime, Utc};

    /// 23:30 UTC on Oct 19: already Oct 20 in Paris and Sydney, still Oct 19 in New York.
    #[test]
    fn today_is_the_day_where_the_payment_was_recorded() {
        let now: DateTime<Utc> = "2026-10-19T23:30:00Z".parse().unwrap();
        let check = |at: &str| check_payment_date(DateTime::parse_from_rfc3339(at).unwrap(), now);
        for today in [
            "2026-10-20T01:30:00+02:00",
            "2026-10-20T23:00:00+02:00",
            "2026-10-20T12:00:00+11:00",
            "2026-10-19T20:00:00-04:00",
            "2026-10-19T23:59:00+00:00",
            "2026-10-20T23:59:00+14:00",
            "2026-01-05T09:00:00+01:00",
        ] {
            assert!(check(today).is_ok(), "{today}");
        }
        for tomorrow in [
            "2026-10-21T00:30:00+02:00",
            "2026-10-20T00:30:00-04:00",
            "2026-10-20T00:00:00+00:00",
            "2026-10-21T00:00:00+14:00",
            "2026-10-20T08:00:00+15:00",
        ] {
            assert!(check(tomorrow).is_err(), "{tomorrow}");
        }
    }
}
