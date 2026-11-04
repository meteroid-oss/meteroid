use crate::StoreResult;
use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::entity_activity::Actor;
use crate::domain::enums::DeclineKind;
use crate::domain::outbox_event::PaymentTransactionEvent;
use crate::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use crate::errors::StoreError;
use crate::repositories::customer_emails::CustomerEmailsInterface;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::repositories::payment_transactions::recompute_invoice_payment_state;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::Services;
use crate::services::payment_reminders::sweep::ReopenedReminders;
use crate::store::{PgConn, Store};
use chrono::Utc;
use common_domain::ids::{
    CustomerId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, SubscriptionId, TenantId,
};
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{InvoicePaymentStatus, InvoiceStatusEnum};
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
pub const DUNNING_RETRY_SCHEDULE_DAYS: [i64; 3] = [3, 5, 7];

/// What the failure transaction did to the invoice; only a real transition tells the customer.
enum FailureOutcome {
    MarkedErrored,
    Skipped,
}

impl Services {
    /// A payment attempt failed or was cancelled without ever settling.
    ///
    /// Distinct from a post-settlement reversal (chargeback / late failure), which
    /// carries `refunded_at` and is handled by `on_payment_transaction_reversed`.
    async fn replayed_attempt(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_transaction_id: PaymentTransactionId,
    ) -> StoreResult<bool> {
        let held_at = {
            let mut conn = self.store.get_conn().await?;
            diesel_models::replay_holds::invoice_delivery_held_at(&mut conn, tenant_id, invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        let Some(held_at) = held_at else {
            return Ok(false);
        };
        let payment = self
            .store
            .get_payment_tx_row(tenant_id, payment_transaction_id)
            .await?;
        Ok(payment
            .processed_at
            .is_some_and(|processed_at| processed_at.and_utc() < held_at))
    }

    pub async fn on_payment_transaction_failed(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        // A top-up owns its failed charge: its document is discarded, never dunned.
        if self.on_wallet_purchase_payment_failed(&event).await? {
            return Ok(());
        }
        let Some(invoice_id) = event.invoice_id else {
            // A checkout-session charge that never reached an invoice: nothing to collect
            // against. The session is released so the customer can retry cleanly.
            if let Some(session_id) = event.checkout_session_id {
                self.release_failed_checkout_session(event.tenant_id, session_id)
                    .await?;
            }
            return Ok(());
        };

        // A replay recorded this attempt's outcome itself: processed before its invoice was held,
        // at a past moment. A later attempt on the same invoice is handled as any other.
        if self
            .replayed_attempt(event.tenant_id, invoice_id, event.payment_transaction_id)
            .await?
        {
            return Ok(());
        }

        // No saved method: the customer's own attempt on a hosted page (Mollie, Stancer). Nothing
        // was charged automatically, so no dunning; the invoice stays payable.
        if event.payment_method_id.is_none() {
            return self
                .release_processing_invoice(event.tenant_id, invoice_id)
                .await;
        }

        // Cancelled is aborted (often on purpose, from the provider dashboard), not declined:
        // retrying it would fight that intent. No dunning; the invoice stays payable.
        if event.status == crate::domain::PaymentStatusEnum::Cancelled {
            self.release_processing_invoice(event.tenant_id, invoice_id)
                .await?;
            // Nothing retries a cancelled system charge: reminders take the invoice over.
            let payment = self
                .store
                .get_payment_tx_row(event.tenant_id, event.payment_transaction_id)
                .await?;
            if payment.initiated_by_customer_id.is_none() {
                self.hand_over_to_reminders(event.tenant_id, invoice_id)
                    .await?;
            }
            return Ok(());
        }

        let outcome = self
            .store
            .transaction(|conn| {
                async move {
                    let invoice =
                        InvoiceRow::select_for_update_by_id(conn, event.tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    // A draft never became a collectible document — this is an abandoned
                    // checkout (card 3DS), not a debt. Leave it for checkout-session cleanup.
                    if invoice.invoice.status != InvoiceStatusEnum::Finalized {
                        return Ok(FailureOutcome::Skipped);
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
                        return Ok(FailureOutcome::Skipped);
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
                        return Ok(FailureOutcome::Skipped);
                    }

                    InvoiceRow::apply_payment_status(
                        conn,
                        invoice_id,
                        event.tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Errored,
                        None,
                    )
                    .await?;

                    let payment = PaymentTransactionRow::get_by_id(
                        conn,
                        event.payment_transaction_id,
                        event.tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    // A system charge used the method a retry would; a portal attempt may not have.
                    // A card refreshed before this handler ran already lifts its decline.
                    let method = match payment.payment_method_id {
                        Some(id) => Some(
                            CustomerPaymentMethodRow::get_by_id(conn, &event.tenant_id, &id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?,
                        ),
                        None => None,
                    };
                    let stopped = is_hard_decline(&payment)
                        && method.is_none_or(|m| !card_refreshed_since(&payment, &m));
                    if payment.initiated_by_customer_id.is_none() && stopped {
                        log::info!(
                            "Invoice {} was hard-declined ({:?}); no retry until the payment method changes",
                            invoice_id,
                            event.decline_kind
                        );
                    } else {
                        self.schedule_next_dunning_attempt(
                            conn,
                            event.tenant_id,
                            invoice_id,
                            invoice.invoice.subscription_id,
                        )
                        .await?;
                    }

                    // The subscription stays active on a failed collection: access is not
                    // revoked by an unpaid invoice in this iteration. Suspension is a
                    // dunning-policy decision that needs tenant-level configuration.
                    Ok(FailureOutcome::MarkedErrored)
                }
                .scope_boxed()
            })
            .await?;

        // D23: `payment_failed` only for a system charge that actually errored the invoice; a
        // customer's own portal attempt already showed them the decline.
        if matches!(outcome, FailureOutcome::MarkedErrored) {
            let payment = self
                .store
                .get_payment_tx_row(event.tenant_id, event.payment_transaction_id)
                .await?;
            if payment.initiated_by_customer_id.is_none() {
                // An off-session charge the bank wants authenticated: the customer must confirm
                // it on the pay page (3DS), which "payment failed" would not tell them.
                let (kind, context, key) =
                    if event.decline_kind == Some(DeclineKind::AuthenticationRequired) {
                        (
                            EmailKind::AuthenticationRequired,
                            EmailContext::AuthenticationRequired {
                                invoice_id,
                                payment_transaction_id: event.payment_transaction_id,
                            },
                            format!("authentication_required:{}", event.payment_transaction_id),
                        )
                    } else {
                        (
                            EmailKind::PaymentFailed,
                            EmailContext::PaymentFailed {
                                invoice_id,
                                payment_transaction_id: event.payment_transaction_id,
                            },
                            format!("payment_failed:{}", event.payment_transaction_id),
                        )
                    };
                self.store
                    .enqueue_customer_email(event.tenant_id, kind, context, key)
                    .await?;
            }
        }
        Ok(())
    }

    /// An invoice the attempt left `Processing` is payable again once nothing else is in flight.
    async fn release_processing_invoice(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if invoice.invoice.payment_status != InvoicePaymentStatus::Processing
                        || PaymentTransactionRow::exists_live_for_invoice(
                            conn, invoice_id, tenant_id,
                        )
                        .await?
                    {
                        return Ok(());
                    }
                    recompute_invoice_payment_state(conn, tenant_id, invoice_id, None).await?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// Automatic collection gave up (its charge was cancelled or dead-lettered): the D4 stamp
    /// is cleared and reminders take over, unless another charge is still in flight.
    pub(crate) async fn hand_over_to_reminders(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if PaymentTransactionRow::exists_live_for_invoice(conn, invoice_id, tenant_id)
                        .await?
                    {
                        return Ok(());
                    }
                    self.store
                        .internal
                        .on_invoice_reopened(
                            conn,
                            tenant_id,
                            invoice_id,
                            false,
                            true,
                            ReopenedReminders::Restart,
                            Utc::now().naive_utc(),
                        )
                        .await
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
        subscription_id: Option<SubscriptionId>,
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
        if has_pending_retry_event(conn, tenant_id, subscription_id, invoice_id).await? {
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
                    created_by: Actor::System,
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

    /// Releases a checkout session whose charge failed before any invoice existed, so the
    /// customer can retry. It goes back to `Created`, not `Cancelled`, so "Open checkout" on the
    /// subscription still works after a declined or expired hosted payment.
    async fn release_failed_checkout_session(
        &self,
        tenant_id: TenantId,
        session_id: common_domain::ids::CheckoutSessionId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    // Idempotent: only an AwaitingPayment session transitions, so a
                    // redelivery finds nothing to do and returns None.
                    CheckoutSessionRow::reopen_after_failed_payment(conn, tenant_id, session_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }
}

/// True when a retry for this invoice is already queued, so a redelivered failure
/// doesn't stack duplicate attempts on the ladder.
async fn has_pending_retry_event(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    invoice_id: InvoiceId,
) -> StoreResult<bool> {
    let pending =
        ScheduledEventRow::get_pending_events_for_subscription(conn, subscription_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    Ok(pending.into_iter().any(|row| {
        matches!(
            serde_json::from_value::<ScheduledEventData>(row.event_data),
            Ok(ScheduledEventData::RetryPayment { invoice_id: queued }) if queued == invoice_id
        )
    }))
}

fn is_hard_decline(failed: &PaymentTransactionRow) -> bool {
    failed
        .decline_kind
        .is_some_and(|kind| DeclineKind::from(kind).is_hard())
}

/// The attempt was hard-declined on this very instrument: charging it again cannot succeed.
pub(crate) fn hard_declined_on(
    failed: &PaymentTransactionRow,
    method: &CustomerPaymentMethodRow,
) -> bool {
    failed.payment_method_id == Some(method.id)
        && is_hard_decline(failed)
        && !card_refreshed_since(failed, method)
}

/// An expired-card decline is lifted by new card details on the same method (Stripe's card
/// updater refreshes the card in place, keeping its id).
fn card_refreshed_since(failed: &PaymentTransactionRow, method: &CustomerPaymentMethodRow) -> bool {
    is_card_expired(failed)
        && method
            .card_refreshed_at
            .is_some_and(|refreshed| refreshed > failed.created_at)
}

fn is_card_expired(failed: &PaymentTransactionRow) -> bool {
    failed
        .decline_kind
        .is_some_and(|kind| DeclineKind::from(kind) == DeclineKind::CardExpired)
}

/// A new default payment method revives the retries a hard decline stopped, since that
/// decline was on the previous instrument.
pub(crate) async fn rearm_hard_declined_retries(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    payment_method_id: CustomerPaymentMethodId,
) -> StoreResult<()> {
    rearm_retries_where(store, conn, tenant_id, customer_id, |failed| {
        failed
            .payment_method_id
            .is_some_and(|declined| declined != payment_method_id)
    })
    .await
}

/// New card details on a method revive the retries an expired-card decline on it stopped.
pub(crate) async fn rearm_refreshed_card_retries(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    payment_method_id: CustomerPaymentMethodId,
) -> StoreResult<()> {
    rearm_retries_where(store, conn, tenant_id, customer_id, |failed| {
        failed.payment_method_id == Some(payment_method_id) && is_card_expired(failed)
    })
    .await
}

/// Schedules a retry now for each errored invoice of the customer whose latest attempt was a
/// hard decline that `revived` lifts, unless a charge or retry is already on its way.
async fn rearm_retries_where(
    store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    revived: impl Fn(&PaymentTransactionRow) -> bool,
) -> StoreResult<()> {
    let latest_failures = PaymentTransactionRow::latest_failed_for_errored_invoices_of_customer(
        conn,
        tenant_id,
        customer_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;

    let mut retries = vec![];
    for (subscription_id, failed) in latest_failures {
        let Some(invoice_id) = failed.invoice_id else {
            continue;
        };
        if !is_hard_decline(&failed) || !revived(&failed) {
            continue;
        }
        if PaymentTransactionRow::exists_live_for_invoice(conn, invoice_id, tenant_id).await?
            || has_pending_retry_event(conn, tenant_id, subscription_id, invoice_id).await?
        {
            continue;
        }
        retries.push(ScheduledEventNew {
            subscription_id,
            tenant_id,
            scheduled_time: Utc::now().naive_utc(),
            event_data: ScheduledEventData::RetryPayment { invoice_id },
            source: "dunning".to_string(),
            created_by: Actor::System,
        });
    }

    if !retries.is_empty() {
        store.schedule_events(conn, retries).await?;
    }
    Ok(())
}
