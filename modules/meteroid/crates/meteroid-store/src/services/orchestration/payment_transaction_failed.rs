use crate::StoreResult;
use crate::domain::outbox_event::PaymentTransactionEvent;
use crate::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use crate::errors::StoreError;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::Services;
use crate::store::PgConn;
use chrono::Utc;
use common_domain::ids::{InvoiceId, TenantId};
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::enums::InvoiceStatusEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRow;
use diesel_models::scheduled_events::ScheduledEventRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Days to wait before each successive retry of a failed invoice payment.
///
/// Length is the retry budget: after the last one the invoice stops being chased
/// automatically and is left for a human (or the customer, via the portal).
/// Spacing is deliberately wider than a card dunning ladder — a direct-debit failure
/// is usually insufficient funds, and re-presenting the next day just fails again.
const DUNNING_RETRY_SCHEDULE_DAYS: [i64; 3] = [3, 5, 7];

impl Services {
    /// A payment attempt failed or was cancelled without ever settling.
    ///
    /// Distinct from a post-settlement reversal (chargeback / late failure), which
    /// carries `refunded_at` and is handled by `on_payment_transaction_reversed`.
    pub async fn on_payment_transaction_failed(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        let Some(invoice_id) = event.invoice_id else {
            // A checkout-session charge that never reached an invoice: nothing to collect
            // against. The session is released so the customer can retry cleanly.
            if let Some(session_id) = event.checkout_session_id {
                self.release_failed_checkout_session(event.tenant_id, session_id)
                    .await?;
            }
            return Ok(());
        };

        self.store
            .transaction(|conn| {
                async move {
                    let invoice =
                        InvoiceRow::select_for_update_by_id(conn, event.tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    // A draft never became a collectible document — this is an abandoned
                    // checkout (card 3DS), not a debt. Leave it for checkout-session cleanup.
                    if invoice.invoice.status != InvoiceStatusEnum::Finalized {
                        return Ok(());
                    }

                    // Recompute rather than trust the event: a redelivery, or a concurrent
                    // settlement of a different attempt, must not push a paid invoice into
                    // Errored. This is the same idempotency contract as the settle handler.
                    let refreshed = InvoiceRow::recompute_amount_due_from_settled_payments(
                        conn,
                        invoice_id,
                        event.tenant_id,
                    )
                    .await?;

                    if refreshed.amount_due <= 0 {
                        return Ok(());
                    }

                    // Another attempt is already in flight (retry raced the failure webhook,
                    // or a customer paid via the portal). It owns the invoice; leave it alone.
                    if PaymentTransactionRow::exists_live_for_invoice(
                        conn,
                        invoice_id,
                        event.tenant_id,
                    )
                    .await?
                    {
                        return Ok(());
                    }

                    InvoiceRow::apply_payment_status(
                        conn,
                        invoice_id,
                        event.tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Errored,
                        None,
                    )
                    .await?;

                    self.schedule_next_dunning_attempt(
                        conn,
                        event.tenant_id,
                        invoice_id,
                        invoice.invoice.subscription_id,
                    )
                    .await?;

                    // The subscription stays active on a failed collection: access is not
                    // revoked by an unpaid invoice in this iteration. Suspension is a
                    // dunning-policy decision that needs tenant-level configuration.
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// Schedules the next automatic retry, if the invoice still has budget left.
    ///
    /// The attempt number is derived from the invoice's own failed attempts rather than
    /// carried on the event, so a redelivered webhook re-derives the same position in the
    /// ladder instead of advancing it.
    async fn schedule_next_dunning_attempt(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        subscription_id: Option<common_domain::ids::SubscriptionId>,
    ) -> StoreResult<()> {
        // Retries are driven by scheduled events, which hang off a subscription. A one-off
        // invoice has nowhere to attach one; it stays Errored and is collected manually.
        let Some(subscription_id) = subscription_id else {
            return Ok(());
        };

        let failed_attempts =
            PaymentTransactionRow::count_failed_for_invoice(conn, invoice_id, tenant_id).await?;

        // `failed_attempts` counts the failure we are reacting to, so attempt 1 picks the
        // first delay in the ladder.
        let Some(delay_days) =
            DUNNING_RETRY_SCHEDULE_DAYS.get(failed_attempts.saturating_sub(1) as usize)
        else {
            log::info!(
                "Invoice {} exhausted its dunning retries after {} attempts; leaving it in collection",
                invoice_id,
                failed_attempts
            );
            return Ok(());
        };

        // Idempotency: a redelivered failure re-derives the same attempt number, and a
        // pending retry for this invoice already covers it.
        if self
            .has_pending_retry_event(conn, tenant_id, subscription_id, invoice_id)
            .await?
        {
            return Ok(());
        }

        let scheduled_time = (Utc::now() + chrono::Duration::days(*delay_days)).naive_utc();

        self.store
            .schedule_events(
                conn,
                vec![ScheduledEventNew {
                    subscription_id,
                    tenant_id,
                    scheduled_time,
                    event_data: ScheduledEventData::RetryPayment { invoice_id },
                    source: "dunning".to_string(),
                    created_by_customer: false,
                }],
            )
            .await?;

        log::info!(
            "Scheduled dunning retry {} for invoice {} in {} days",
            failed_attempts,
            invoice_id,
            delay_days
        );

        Ok(())
    }

    /// True when a retry for this invoice is already queued, so a redelivered failure
    /// doesn't stack duplicate attempts on the ladder.
    async fn has_pending_retry_event(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
        invoice_id: InvoiceId,
    ) -> StoreResult<bool> {
        let pending = ScheduledEventRow::get_pending_events_for_subscription(
            conn,
            subscription_id,
            &tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(pending.into_iter().any(|row| {
            matches!(
                serde_json::from_value::<ScheduledEventData>(row.event_data),
                Ok(ScheduledEventData::RetryPayment { invoice_id: queued }) if queued == invoice_id
            )
        }))
    }

    /// Releases a checkout session whose charge failed before any invoice existed, so the
    /// customer can start over instead of being stuck on `AwaitingPayment`.
    async fn release_failed_checkout_session(
        &self,
        tenant_id: TenantId,
        session_id: common_domain::ids::CheckoutSessionId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    // Idempotent: only Created/AwaitingPayment sessions transition, so a
                    // redelivery finds nothing to do and returns None.
                    CheckoutSessionRow::mark_cancelled(conn, tenant_id, session_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }
}
