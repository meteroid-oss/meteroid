use crate::StoreResult;
use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::dunning_policy::{DunningPolicy, ExhaustedAction};
use crate::domain::entity_activity::Actor;
use crate::domain::enums::DeclineKind;
use crate::domain::outbox_event::PaymentTransactionEvent;
use crate::errors::StoreError;
use crate::repositories::customer_emails::CustomerEmailsInterface;
use crate::repositories::invoice_account::{self, is_issued};
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use crate::services::Services;
use crate::services::payment_reminders::sweep::ReopenedReminders;
use crate::store::{PgConn, Store};
use billing_core::settlement::{Charge, PaymentStatus};
use common_domain::ids::{
    CustomerId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, SubscriptionId, TenantId,
};
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::dunning_policy::DunningPolicyRow;
use diesel_models::enums::InvoiceStatusEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Days to wait before each successive retry of a failed invoice payment, unless the tenant's
/// dunning policy says otherwise.
///
/// Length is the retry budget: after the last one the invoice stops being chased
/// automatically and is left for a human (or the customer, via the portal).
/// Spacing is deliberately wider than a card dunning ladder — a direct-debit failure
/// is usually insufficient funds, and re-presenting the next day just fails again.
pub const DUNNING_RETRY_SCHEDULE_DAYS: [i64; 3] = [3, 5, 7];

/// What the failure transaction did to the invoice; only a real transition tells the customer.
enum FailureOutcome {
    /// `cancel`: the retries ran out and the policy cancels this subscription.
    MarkedErrored {
        cancel: Option<SubscriptionId>,
    },
    Skipped,
}

/// Where the failure left the invoice's automatic retries.
enum Retries {
    Scheduled,
    Exhausted(DunningPolicy),
    NotApplicable,
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
        // retrying it would fight that intent. No dunning; the invoice stays payable. An
        // unconfirmed charge may have gone through, so it is neither retried nor reported failed.
        if event.status == crate::domain::PaymentStatusEnum::Cancelled
            || event.decline_kind == Some(DeclineKind::Unconfirmed)
        {
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

                    // Settled from the account rather than the event: a redelivery, or a
                    // concurrent settlement of a different attempt, never errors a paid invoice.
                    let settled =
                        invoice_account::settle_tx(conn, event.tenant_id, invoice_id, None).await?;

                    // Paid, or another attempt is in flight (a retry raced the failure webhook, or
                    // the customer paid in the portal) and owns the invoice: leave it alone.
                    if !matches!(settled.state.charge(), Charge::Due(_))
                        || settled.state.status != PaymentStatus::Errored
                    {
                        return Ok(FailureOutcome::Skipped);
                    }

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
                        && method
                            .as_ref()
                            .is_none_or(|m| !card_refreshed_since(&payment, m));
                    if payment.initiated_by_customer_id.is_none() && stopped {
                        log::info!(
                            "Invoice {} was hard-declined ({:?}); no retry until the payment method changes",
                            invoice_id,
                            event.decline_kind
                        );
                        if invoice.invoice.subscription_id.is_some() {
                            let policy: DunningPolicy = DunningPolicyRow::find(conn, event.tenant_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?
                                .map(Into::into)
                                .unwrap_or_default();
                            if let Some(days) = policy.hard_decline_grace_days() {
                                let at = common_clock::now().naive_utc()
                                    + chrono::Duration::days(i64::from(days));
                                InvoiceRow::set_dunning_cancel_at(
                                    conn,
                                    event.tenant_id,
                                    invoice_id,
                                    Some(at),
                                )
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                                diesel_models::billing_controller::BillingControllerRow::wake_by(
                                    conn,
                                    event.tenant_id,
                                    invoice.invoice.customer_id,
                                    at.and_utc(),
                                )
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                            }
                        }
                        return Ok(FailureOutcome::MarkedErrored { cancel: None });
                    }
                    let retries = self
                        .schedule_next_dunning_attempt(
                            conn,
                            event.tenant_id,
                            invoice_id,
                            invoice.invoice.subscription_id,
                            &payment,
                            method.as_ref(),
                        )
                        .await?;

                    // Access stays on while retries run; once they are spent, the tenant's policy
                    // decides whether the subscription goes.
                    let cancel = match retries {
                        Retries::Exhausted(policy)
                            if payment.initiated_by_customer_id.is_none()
                                && policy.exhausted_action
                                    == ExhaustedAction::CancelSubscription =>
                        {
                            invoice.invoice.subscription_id
                        }
                        _ => None,
                    };
                    Ok(FailureOutcome::MarkedErrored { cancel })
                }
                .scope_boxed()
            })
            .await?;

        if let FailureOutcome::MarkedErrored {
            cancel: Some(subscription_id),
        } = outcome
        {
            // Boxed so this handler holds a pointer, not the cancellation's state machine.
            Box::pin(self.cancel_after_exhausted_retries(
                event.tenant_id,
                subscription_id,
                invoice_id,
            ))
            .await?;
        }

        // `payment_failed` only for a system charge that actually errored the invoice; a
        // customer's own portal attempt already showed them the decline.
        if matches!(outcome, FailureOutcome::MarkedErrored { .. }) {
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
                    // The attempt is recorded: the account no longer counts it in flight.
                    if is_issued(&invoice.invoice) {
                        invoice_account::settle_tx(conn, tenant_id, invoice_id, None).await?;
                    }
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// Automatic collection gave up (its charge was cancelled or dead-lettered): the auto-charge
    /// stamp is cleared and reminders take over, unless another charge is still in flight.
    pub(crate) async fn hand_over_to_reminders(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .invoice;
                    if !matches!(
                        invoice_account::charge_tx(conn, &invoice).await?,
                        Charge::Due(_)
                    ) {
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
                            common_clock::now().naive_utc(),
                        )
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    /// The retries ran out and the tenant's policy cancels the subscription: at once, the
    /// invoice staying due. Checked again under the customer lock, a payment landing meanwhile or
    /// the subscription ending already leaves it alone.
    pub(in crate::services) async fn cancel_after_exhausted_retries(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        invoice_id: InvoiceId,
    ) -> StoreResult<()> {
        let cancelled = Box::pin(self.cancel_subscription_unless(
            Actor::System,
            subscription_id,
            tenant_id,
            Some(format!(
                "Payment failed after every retry (invoice {invoice_id})"
            )),
            CancellationEffectiveAt::Immediate,
            common_clock::now(),
            Some(invoice_id),
            false,
        ))
        .await?;
        if cancelled.is_some() {
            log::info!(
                "Invoice {invoice_id} exhausted its payment retries; cancelled subscription {subscription_id}"
            );
        }
        Ok(())
    }

    /// Sets the invoice's next automatic payment attempt, if it still has budget left.
    ///
    /// The attempt number is derived from the invoice's own failed automatic attempts on the
    /// instrument charged, so a redelivered webhook re-derives the same position in the ladder
    /// instead of advancing it, and a new or refreshed card starts it again.
    #[allow(clippy::too_many_arguments)]
    async fn schedule_next_dunning_attempt(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        subscription_id: Option<SubscriptionId>,
        failed: &PaymentTransactionRow,
        method: Option<&CustomerPaymentMethodRow>,
    ) -> StoreResult<Retries> {
        // A retry charges the subscription's method: a one-off invoice stays Errored and is
        // collected manually.
        if subscription_id.is_none() {
            return Ok(Retries::NotApplicable);
        }

        let failed_attempts = match failed.payment_method_id {
            Some(method_id) => {
                PaymentTransactionRow::count_dunning_failures(
                    conn,
                    invoice_id,
                    tenant_id,
                    method_id,
                    method
                        .and_then(|m| m.card_refreshed_at)
                        .map(|at| at.naive_utc()),
                )
                .await?
            }
            None => {
                PaymentTransactionRow::count_failed_for_invoice(conn, invoice_id, tenant_id).await?
            }
        };
        let policy: DunningPolicy = DunningPolicyRow::find(conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .map(Into::into)
            .unwrap_or_default();

        // `failed_attempts` counts the failure we are reacting to, so attempt 1 picks the
        // first delay in the ladder.
        let Some(delay_days) = policy.delay_before(failed_attempts as u32) else {
            log::info!(
                "Invoice {} exhausted its dunning retries after {} attempts",
                invoice_id,
                failed_attempts
            );
            return Ok(Retries::Exhausted(policy));
        };

        // Idempotency: a redelivered failure re-derives the same attempt number, and an attempt
        // already set covers it.
        let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if invoice.next_payment_attempt_at.is_some() {
            return Ok(Retries::Scheduled);
        }

        let attempt_at = common_clock::now() + chrono::Duration::days(i64::from(delay_days));
        set_payment_attempt(conn, tenant_id, invoice.customer_id, invoice_id, attempt_at).await?;

        log::info!(
            "Scheduled dunning retry {} for invoice {} in {} days",
            failed_attempts,
            invoice_id,
            delay_days
        );

        Ok(Retries::Scheduled)
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

/// The invoice's next payment attempt at `at`, its customer's controller woken for it.
async fn set_payment_attempt(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    invoice_id: InvoiceId,
    at: chrono::DateTime<chrono::Utc>,
) -> StoreResult<()> {
    InvoiceRow::set_next_payment_attempt_at(conn, tenant_id, invoice_id, Some(at.naive_utc()))
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    diesel_models::billing_controller::BillingControllerRow::wake_by(
        conn,
        tenant_id,
        customer_id,
        at,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)
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
    _store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    payment_method_id: CustomerPaymentMethodId,
) -> StoreResult<()> {
    rearm_retries_where(conn, tenant_id, customer_id, |failed| {
        failed
            .payment_method_id
            .is_some_and(|declined| declined != payment_method_id)
    })
    .await
}

/// New card details on a method revive the retries an expired-card decline on it stopped.
pub(crate) async fn rearm_refreshed_card_retries(
    _store: &Store,
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    payment_method_id: CustomerPaymentMethodId,
) -> StoreResult<()> {
    rearm_retries_where(conn, tenant_id, customer_id, |failed| {
        failed.payment_method_id == Some(payment_method_id) && is_card_expired(failed)
    })
    .await
}

/// Sets a payment attempt now on each errored invoice of the customer whose latest attempt was
/// a hard decline that `revived` lifts, unless a charge or an attempt is already on its way.
async fn rearm_retries_where(
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

    for (_, failed) in latest_failures {
        let Some(invoice_id) = failed.invoice_id else {
            continue;
        };
        if !is_hard_decline(&failed) || !revived(&failed) {
            continue;
        }
        let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if !matches!(
            invoice_account::charge_tx(conn, &invoice).await?,
            Charge::Due(_)
        ) || invoice.next_payment_attempt_at.is_some()
        {
            continue;
        }
        set_payment_attempt(
            conn,
            tenant_id,
            customer_id,
            invoice_id,
            common_clock::now(),
        )
        .await?;
        InvoiceRow::set_dunning_cancel_at(conn, tenant_id, invoice_id, None)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok(())
}

/// Whether `invoice_id` is still due with nothing in flight and no attempt left, and the
/// subscription still running with no cancellation ahead, the invoice locked under `lock`. A
/// voided or written-off invoice ends nothing.
pub(in crate::services) async fn retries_exhausted_tx(
    conn: &mut PgConn,
    lock: &diesel_models::lock::CustomerLock,
    details: &crate::domain::SubscriptionDetails,
    invoice_id: InvoiceId,
) -> StoreResult<bool> {
    if details.subscription.status.is_terminal()
        || details.pending_events.iter().any(|e| {
            e.event_type == crate::domain::enums::ScheduledEventTypeEnum::CancelSubscription
        })
    {
        return Ok(false);
    }
    let invoice = InvoiceRow::lock_by_id(conn, lock, invoice_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(invoice.status == InvoiceStatusEnum::Finalized
        && invoice.next_payment_attempt_at.is_none()
        && matches!(
            invoice_account::charge_tx(conn, &invoice).await?,
            Charge::Due(_)
        ))
}
