use crate::StoreResult;
use crate::domain::credit::ApiIdempotency;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use crate::domain::payment_transactions::PaymentTransaction;
use crate::errors::StoreError;
use crate::repositories::InvoiceInterface;
use crate::repositories::api_idempotency::{ClaimOutcome, claim_tx};
use crate::services::{InvoiceDelivery, Services};
use chrono::{DateTime, FixedOffset, Utc};
use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId, TenantId};
use common_utils::decimals::ToSubunit;
use diesel_models::PgConn;
use diesel_models::enums::{InvoiceStatusEnum, PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use error_stack::Report;
use rust_decimal::Decimal;
use scoped_futures::ScopedFutureExt;

pub struct ManualInvoicePayment {
    pub amount: i64,
    pub paid_at: DateTime<Utc>,
    pub reference: Option<String>,
}

#[derive(Clone, Copy)]
pub enum ManualInvoicePaymentKind {
    Record,
    MarkPaid,
}

impl Services {
    /// Records a bank transfer, cash, or check through the admin API. The worker settles the invoice.
    pub async fn add_manual_payment_transaction(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        amount: Decimal,
        payment_date: DateTime<FixedOffset>,
        reference: Option<String>,
    ) -> StoreResult<PaymentTransaction> {
        self.store
            .transaction(|conn| {
                let (actor, reference) = (&actor, &reference);
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await?
                        .invoice;
                    check_manual_payment_invoice(&invoice, ManualInvoicePaymentKind::Record)?;
                    if amount <= Decimal::ZERO {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Payment amount must be positive".into(),
                        )));
                    }
                    check_payment_date(payment_date, common_clock::now())?;
                    let payment = ManualInvoicePayment {
                        amount: manual_payment_minor_units(amount, &invoice.currency)?,
                        paid_at: payment_date.to_utc(),
                        reference: reference.clone(),
                    };
                    self.record_manual_payment_tx(
                        conn,
                        actor,
                        &invoice,
                        &payment,
                        ManualInvoicePaymentKind::Record,
                        invoice.amount_due,
                        PaymentTransactionId::new(),
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }

    /// Records a manual payment for the full balance through the admin API.
    pub async fn mark_invoice_as_paid(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        total_amount: Decimal,
        payment_date: DateTime<FixedOffset>,
        reference: Option<String>,
    ) -> StoreResult<crate::domain::DetailedInvoice> {
        self.store
            .transaction(|conn| {
                let (actor, reference) = (&actor, &reference);
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await?
                        .invoice;
                    check_manual_payment_invoice(&invoice, ManualInvoicePaymentKind::MarkPaid)?;
                    check_payment_date(payment_date, common_clock::now())?;
                    let payment = ManualInvoicePayment {
                        amount: manual_payment_minor_units(total_amount, &invoice.currency)?,
                        paid_at: payment_date.to_utc(),
                        reference: reference.clone(),
                    };
                    self.record_manual_payment_tx(
                        conn,
                        actor,
                        &invoice,
                        &payment,
                        ManualInvoicePaymentKind::MarkPaid,
                        invoice.amount_due,
                        PaymentTransactionId::new(),
                    )
                    .await?;
                    self.store
                        .get_detailed_invoice_by_id_with_conn(conn, tenant_id, invoice_id)
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    /// The public API claims the key, records the payment, and settles the invoice in one
    /// transaction. A retry returns the addressed payment without recording or settling it again.
    async fn record_manual_invoice_payment(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment: ManualInvoicePayment,
        kind: ManualInvoicePaymentKind,
        identity: &ApiIdempotency,
    ) -> StoreResult<PaymentTransaction> {
        let transaction_id = PaymentTransactionId::from(identity.derived_id(tenant_id));
        self.store
            .transaction(|conn| {
                let (actor, payment) = (&actor, &payment);
                async move {
                    if let ClaimOutcome::Existing = claim_tx(conn, tenant_id, identity).await? {
                        return Ok(PaymentTransactionRow::get_by_id(
                            conn,
                            transaction_id,
                            tenant_id,
                        )
                        .await?
                        .into());
                    }

                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await?
                        .invoice;
                    if payment.amount <= 0 {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "Payment amount must be positive".into(),
                        )));
                    }
                    check_manual_payment_invoice(&invoice, kind)?;
                    check_paid_at(payment.paid_at, common_clock::now())?;
                    // Count money already recorded by the admin API even if its settlement event
                    // is still in the queue.
                    let amount_due = InvoiceRow::collections(conn, invoice_id, tenant_id)
                        .await?
                        .amount_due();
                    if PaymentTransactionRow::exists_in_flight_for_invoice(
                        conn, invoice_id, tenant_id,
                    )
                    .await?
                    {
                        return Err(Report::new(StoreError::InvalidArgument(
                            "A payment of this invoice is already in progress".to_string(),
                        )));
                    }
                    let transaction = self
                        .record_manual_payment_tx(
                            conn,
                            actor,
                            &invoice,
                            payment,
                            kind,
                            amount_due,
                            transaction_id,
                        )
                        .await?;
                    self.on_invoice_payment_settled_tx(
                        conn,
                        &transaction.clone().into(),
                        invoice_id,
                        common_clock::now(),
                    )
                    .await?;
                    Ok(transaction)
                }
                .scope_boxed()
            })
            .await
    }

    async fn record_manual_payment_tx(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        invoice: &InvoiceRow,
        payment: &ManualInvoicePayment,
        kind: ManualInvoicePaymentKind,
        amount_due: i64,
        transaction_id: PaymentTransactionId,
    ) -> StoreResult<PaymentTransaction> {
        let amount = payment.amount;
        match kind {
            ManualInvoicePaymentKind::Record if amount > amount_due => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Payment amount ({amount}) exceeds invoice amount due ({amount_due})"
                ))));
            }
            ManualInvoicePaymentKind::MarkPaid if amount != amount_due => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Payment amount ({amount}) must match invoice amount due ({amount_due})"
                ))));
            }
            _ => {}
        }

        let transaction: PaymentTransaction = PaymentTransactionRowNew {
            id: transaction_id,
            tenant_id: invoice.tenant_id,
            invoice_id: Some(invoice.id),
            provider_transaction_id: payment.reference.clone(),
            amount,
            currency: invoice.currency.clone(),
            // The money was received outside Meteroid, so there is no saved payment method.
            payment_method_id: None,
            status: PaymentStatusEnum::Settled,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: Some(payment.paid_at.naive_utc()),
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
        .await?
        .into();
        self.store
            .internal
            .record_outbox_batch_tx(
                conn,
                invoice.tenant_id,
                actor,
                vec![OutboxEvent::payment_transaction_saved(
                    transaction.clone().into(),
                )],
            )
            .await?;
        Ok(transaction)
    }
}

fn check_manual_payment_invoice(
    invoice: &InvoiceRow,
    kind: ManualInvoicePaymentKind,
) -> StoreResult<()> {
    let (action, consolidated_error) = match kind {
        ManualInvoicePaymentKind::Record => (
            "add manual payments",
            "Cannot add a payment to an invoice merged into a consolidated parent",
        ),
        ManualInvoicePaymentKind::MarkPaid => (
            "mark as paid",
            "Cannot mark an invoice merged into a consolidated parent as paid",
        ),
    };
    if invoice.status != InvoiceStatusEnum::Finalized {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "Invoice must be in Finalized status to {action}"
        ))));
    }
    // A consolidated child is billed via its parent; payments belong on the parent.
    if invoice.consolidated_into_invoice_id.is_some() {
        return Err(Report::new(StoreError::InvalidArgument(
            consolidated_error.into(),
        )));
    }
    Ok(())
}

fn manual_payment_minor_units(amount: Decimal, currency: &str) -> StoreResult<i64> {
    let exponent = meteroid_money::iso::find(currency)
        .ok_or_else(|| Report::new(StoreError::InvalidArgument("Invalid currency".into())))?
        .exponent;
    amount.to_subunit_opt(exponent as u8).ok_or_else(|| {
        Report::new(StoreError::InvalidArgument(format!(
            "Invalid amount for currency {currency}"
        )))
    })
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

/// A payment reported through the public API carries no time zone to say what its today is.
/// Wherever the caller is, any moment of their today is less than a day away.
fn check_paid_at(paid_at: DateTime<Utc>, now: DateTime<Utc>) -> StoreResult<()> {
    if paid_at > now + chrono::Duration::days(1) {
        return Err(Report::new(StoreError::InvalidArgument(
            "A payment cannot be dated more than a day ahead".to_string(),
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
        if paid_at > common_clock::now() {
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
                    check_payment_date(paid_at.fixed_offset(), common_clock::now())?;
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
    pub async fn record_manual_invoice_payment(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment: ManualInvoicePayment,
        kind: ManualInvoicePaymentKind,
        identity: &ApiIdempotency,
    ) -> StoreResult<PaymentTransaction> {
        self.services
            .record_manual_invoice_payment(actor, tenant_id, invoice_id, payment, kind, identity)
            .await
    }

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
    use super::{check_paid_at, check_payment_date};
    use chrono::{DateTime, Duration, Utc};

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

    #[test]
    fn a_reported_payment_is_dated_at_most_a_day_ahead() {
        let now: DateTime<Utc> = "2026-10-19T23:30:00Z".parse().unwrap();
        let check = |at: DateTime<Utc>| check_paid_at(at, now);
        assert!(check(now - Duration::days(400)).is_ok());
        assert!(check(now).is_ok());
        // The end of today at UTC+14, the time zone furthest ahead.
        assert!(check("2026-10-20T09:59:59Z".parse().unwrap()).is_ok());
        assert!(check(now + Duration::days(1)).is_ok());
        assert!(check(now + Duration::days(1) + Duration::seconds(1)).is_err());
    }
}
