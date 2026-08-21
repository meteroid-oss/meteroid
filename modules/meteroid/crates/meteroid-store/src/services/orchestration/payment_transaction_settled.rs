use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use crate::domain::{Invoice, InvoicePaymentStatus, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::services::subscriptions::PaymentActivationParams;
use crate::services::subscriptions::utils::is_paid_trial;
use crate::utils::periods::calculate_advance_period_range;
use chrono::{Datelike, Utc};
use common_domain::ids::BaseId;
use diesel_models::checkout_sessions::CheckoutSessionRow;
use diesel_models::enums::{CycleActionEnum, SubscriptionActivationConditionEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::PaymentTransactionRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// Which lifecycle moment is materializing a checkout into a subscription+invoice.
/// Selects only the invoice-settlement treatment; every other step is identical.
#[derive(Debug, Clone, Copy)]
enum CheckoutPaymentPhase {
    /// `payments.confirmed`: funds landed. Invoice → Paid, apply the amount.
    Settled,
    /// `billing_requests.fulfilled`: mandate signed + first payment submitted but
    /// not yet settled. Invoice → Processing, link the still-Pending tx, amount_due
    /// stays full until the settlement webhook.
    AcceptedInFlight,
}

impl Services {
    pub async fn on_payment_transaction_settled(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        match (event.invoice_id, event.checkout_session_id) {
            (Some(invoice_id), _) => self.on_invoice_payment_settled(event, invoice_id).await,
            (None, Some(checkout_session_id)) => {
                self.on_checkout_payment_settled(event, checkout_session_id)
                    .await
            }
            (None, None) => {
                log::warn!(
                    "Payment transaction {} has neither invoice_id nor checkout_session_id",
                    event.payment_transaction_id
                );
                Err(Report::new(StoreError::InvalidArgument(
                    "Payment transaction must have either invoice_id or checkout_session_id"
                        .to_string(),
                )))
            }
        }
    }

    /// Handle payment settlement for a standard invoice payment
    async fn on_invoice_payment_settled(
        &self,
        event: PaymentTransactionEvent,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    let invoice =
                        InvoiceRow::select_for_update_by_id(conn, event.tenant_id, invoice_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                    // Attribute the settlement to the customer who initiated it (portal
                    // payment); a system auto-charge leaves the initiator null → System.
                    let payment = PaymentTransactionRow::get_by_id(
                        conn,
                        event.payment_transaction_id,
                        event.tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    let actor = payment
                        .initiated_by_customer_id
                        .map(|id| Actor::Customer { id })
                        .unwrap_or(Actor::System);

                    let subscription_id = invoice.invoice.subscription_id;

                    let should_finalize =
                        invoice.invoice.status == diesel_models::enums::InvoiceStatusEnum::Draft;

                    // if the invoice is not finalized nor void, finalize it (no line data refresh)
                    if should_finalize {
                        self.finalize_invoice_tx(conn, &actor, invoice.invoice.id, invoice.invoice.tenant_id,
                            false,
                            &None,
                        )
                        .await?;
                    }

                    // Idempotently recompute amount_due from Σ settled payments rather than
                    // blindly subtracting event.amount: a pgmq redelivery of the same settle
                    // event (commit-then-delete window) must be a no-op, never driving a paid
                    // invoice negative or flipping it back to PartiallyPaid.
                    let res = InvoiceRow::recompute_amount_due_from_settled_payments(
                        conn,
                        invoice_id,
                        event.tenant_id,
                    )
                    .await?;

                    let completed = res.amount_due == 0;

                    if completed {
                        let invoice: Invoice = res.try_into()?;

                        if invoice.payment_status != InvoicePaymentStatus::Paid {
                            InvoiceRow::apply_payment_status(
                                conn,
                                invoice_id,
                                event.tenant_id,
                                diesel_models::enums::InvoicePaymentStatus::Paid,
                                event.processed_at,
                            )
                            .await?;

                            self.store
                                .internal
                                .record_outbox_batch_tx(
                                    conn,
                                    event.tenant_id,
                                    &Actor::System,
                                    vec![OutboxEvent::invoice_paid((&invoice).into())],
                                )
                                .await?;
                        }

                        // Apply deferred plan change if the transaction carries a pending plan version.
                        if let (Some(sub_id), Some(target_pvid)) =
                            (invoice.subscription_id, event.pending_plan_version_id)
                        {
                            log::info!(
                                "Applying deferred plan change for subscription {} to plan_version {}",
                                sub_id,
                                target_pvid,
                            );

                            // Use current date instead of invoice_date: async payments
                            // (SEPA) can settle days later, after the billing period rolls over.
                            let change_date = Utc::now().date_naive();
                            let prepared = self
                                .prepare_plan_change_tx(
                                    conn,
                                    sub_id,
                                    event.tenant_id,
                                    target_pvid,
                                    &[],
                                    change_date,
                                )
                                .await?;

                            self.execute_plan_change_tx(
                                conn,
                                &prepared,
                                sub_id,
                                event.tenant_id,
                                target_pvid,
                                change_date,
                            )
                            .await?;
                        }

                        // Activate subscription if pending checkout (TrialExpired activation is handled by on_invoice_paid)
                        if let Some(subscription_id) = subscription_id.as_ref() {
                            let subscription = SubscriptionRow::get_subscription_by_id(
                                conn,
                                &event.tenant_id,
                                *subscription_id,
                            )
                            .await?;

                            let should_activate = subscription.subscription.activated_at.is_none()
                                && subscription.subscription.activation_condition
                                    == SubscriptionActivationConditionEnum::OnCheckout;

                            if should_activate {
                                let billing_start_date = subscription
                                    .subscription
                                    .billing_start_date
                                    .unwrap_or(chrono::Utc::now().date_naive());

                                let current_period_start;
                                let current_period_end;
                                let next_cycle_action;
                                let mut cycle_index = None;
                                let status;

                                if let Some(trial_duration) = subscription.subscription.trial_duration {
                                    status = SubscriptionStatusEnum::TrialActive;
                                    current_period_start = billing_start_date;
                                    current_period_end = Some(
                                        current_period_start
                                            + chrono::Duration::days(i64::from(trial_duration)),
                                    );
                                    next_cycle_action = Some(CycleActionEnum::EndTrial);
                                } else {
                                    let range = calculate_advance_period_range(
                                        billing_start_date,
                                        subscription.subscription.billing_day_anchor as u32,
                                        true,
                                        &subscription.subscription.period.into(),
                                    );

                                    status = SubscriptionStatusEnum::Active;
                                    cycle_index = Some(0);
                                    current_period_start = range.start;
                                    current_period_end = Some(range.end);
                                    next_cycle_action = Some(CycleActionEnum::RenewSubscription);
                                }

                                // TODO send a subscription_activated event
                                SubscriptionRow::activate_subscription(
                                    conn,
                                    subscription_id,
                                    &event.tenant_id,
                                    current_period_start,
                                    current_period_end,
                                    next_cycle_action,
                                    cycle_index,
                                    status.into(),
                                )
                                .await?;
                            }

                            // The charge on this path is linked to the invoice, so the
                            // checkout branch below never runs for it and the session would
                            // stay AwaitingPayment forever (card 3DS completed out-of-band).
                            // Idempotent: re-running just rewrites the same terminal state.
                            if let Some(session_id) = event.checkout_session_id {
                                CheckoutSessionRow::mark_completed(
                                    conn,
                                    event.tenant_id,
                                    session_id,
                                    *subscription_id,
                                    Utc::now(),
                                )
                                .await?;
                            }
                        }
                    } else {
                        // Derive the label the same way the reversal paths do: a
                        // stale settlement event replayed after a full reversal
                        // recomputes amount_due == total, which is Unpaid, not
                        // PartiallyPaid.
                        let payment_status = if res.amount_due >= res.total {
                            diesel_models::enums::InvoicePaymentStatus::Unpaid
                        } else {
                            diesel_models::enums::InvoicePaymentStatus::PartiallyPaid
                        };
                        InvoiceRow::apply_payment_status(
                            conn,
                            invoice_id,
                            event.tenant_id,
                            payment_status,
                            event.processed_at,
                        )
                        .await?;
                    }

                    // TODO payment receipt

                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        Ok(())
    }

    /// A previously-SETTLED payment was clawed back by the bank (GoCardless
    /// `charged_back` / `late_failure_settled`). Reverse it on the invoice: put
    /// the reclaimed amount back into `amount_due` and downgrade the payment
    /// status. The invoice stays Finalized so it re-enters collection.
    ///
    /// Recomputes `amount_due` / `payment_status` from the transactions that are
    /// STILL settled (the clawed-back one is already `Failed`, so it's excluded),
    /// rather than blindly decrementing. That makes it (a) idempotent — a
    /// redelivered event derives the same value from the terminal transaction
    /// states, so nothing double-reverses — and (b) correct for invoices covered
    /// by more than one payment (each clawback removes exactly its own amount).
    pub async fn on_payment_transaction_reversed(
        &self,
        event: PaymentTransactionEvent,
    ) -> StoreResult<()> {
        let Some(invoice_id) = event.invoice_id else {
            log::warn!(
                "Reversed payment transaction {} has no invoice_id (checkout reversal unsupported); \
                 manual reconciliation required",
                event.payment_transaction_id
            );
            return Ok(());
        };

        log::error!(
            "Payment transaction {} reversed (funds reclaimed by bank); reversing invoice {} by {} {}",
            event.payment_transaction_id,
            invoice_id,
            event.amount,
            event.currency
        );

        self.store
            .transaction(|conn| {
                async move {
                    use diesel_models::enums::InvoicePaymentStatus;

                    // Lock the invoice, then re-derive amount_due + payment status
                    // from the still-settled payments via the SAME recompute the
                    // webhook reversal path (`reverse_transaction_tx`) uses, so the
                    // two reversal paths can never disagree on the reopened total.
                    InvoiceRow::select_for_update_by_id(conn, event.tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let recomputed = InvoiceRow::recompute_amount_due_from_settled_payments(
                        conn,
                        invoice_id,
                        event.tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let new_status = if recomputed.amount_due <= 0 {
                        InvoicePaymentStatus::Paid
                    } else if recomputed.amount_due >= recomputed.total {
                        InvoicePaymentStatus::Unpaid
                    } else {
                        InvoicePaymentStatus::PartiallyPaid
                    };
                    if new_status != recomputed.payment_status {
                        InvoiceRow::apply_payment_status(
                            conn,
                            invoice_id,
                            event.tenant_id,
                            new_status,
                            event.refunded_at,
                        )
                        .await?;
                    }

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }

    /// Handle payment settlement for a checkout session (async payment that was pending).
    ///
    /// Reached when the local checkout transaction settles WITHOUT ever having been
    /// linked to an invoice (the deferred charge-first path: an existing mandate
    /// charged off-session at confirm time, held Pending until `payments.confirmed`).
    /// Subscription/invoice are created here, at settlement, and marked Paid.
    ///
    /// A hosted combined mandate+payment checkout does NOT reach this: it is
    /// materialized in-flight on `billing_requests.fulfilled`
    /// ([`Self::on_checkout_payment_accepted_in_flight`]), which links the tx to an
    /// invoice — so its later settlement routes through `on_invoice_payment_settled`.
    async fn on_checkout_payment_settled(
        &self,
        event: PaymentTransactionEvent,
        checkout_session_id: common_domain::ids::CheckoutSessionId,
    ) -> StoreResult<()> {
        self.resolve_checkout_payment(event, checkout_session_id, CheckoutPaymentPhase::Settled)
            .await
    }

    /// Materialize a hosted-checkout payment that the provider has ACCEPTED but
    /// not yet settled (GoCardless `billing_requests.fulfilled` for a combined
    /// mandate+payment Billing Request). Creates/activates the subscription and
    /// finalizes the first invoice as `Processing`, linking the pre-created
    /// Pending transaction. The subsequent `payments.confirmed` flips it to Paid
    /// via `on_invoice_payment_settled` (the tx now carries the invoice id).
    pub async fn on_checkout_payment_accepted_in_flight(
        &self,
        event: PaymentTransactionEvent,
        checkout_session_id: common_domain::ids::CheckoutSessionId,
    ) -> StoreResult<()> {
        self.resolve_checkout_payment(
            event,
            checkout_session_id,
            CheckoutPaymentPhase::AcceptedInFlight,
        )
        .await
    }

    /// Shared body for the two checkout-materialization phases. `phase` decides
    /// only how the first invoice is settled (Paid + apply amount vs Processing +
    /// link the still-Pending tx); subscription creation/activation and
    /// session-completion are identical, so the two phases stay in lockstep.
    async fn resolve_checkout_payment(
        &self,
        event: PaymentTransactionEvent,
        checkout_session_id: common_domain::ids::CheckoutSessionId,
        phase: CheckoutPaymentPhase,
    ) -> StoreResult<()> {
        use crate::domain::checkout_sessions::CheckoutType;
        use crate::services::InvoiceBillingMode;
        use crate::services::checkout_completion::DirectChargeResult;
        use diesel_models::customers::CustomerRow;

        self.store
            .transaction(|conn| {
                async move {
                    // FOR UPDATE: this is the single serialization point for
                    // materializing a checkout. Two concurrent deliveries (a
                    // redelivered `billing_requests.fulfilled`, or fulfilled racing
                    // the confirmed-won-race Settled path) would otherwise both read
                    // `is_completed() == false` and each create a subscription +
                    // invoice from one checkout. The lock makes the second block,
                    // then observe Completed below and no-op.
                    let session: crate::domain::checkout_sessions::CheckoutSession =
                        CheckoutSessionRow::get_by_id_for_update(
                            conn,
                            event.tenant_id,
                            checkout_session_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into();

                    // Rebuild on the day the charge was PRICED — the checkout tx was
                    // created by the confirm/initiate that validated the displayed
                    // amount — not on webhook day. A debit that settles days later must
                    // reproduce the same dated subscription/invoice, or the amount guard
                    // trips with the money already collected.
                    let priced_on = PaymentTransactionRow::get_by_id(
                        conn,
                        event.payment_transaction_id,
                        event.tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .created_at
                    .date_naive();

                    if session.is_completed() {
                        log::warn!(
                            "Checkout session {} already completed, ignoring duplicate webhook",
                            checkout_session_id
                        );
                        return Ok(());
                    }

                    // Do NOT block on expiry here: reaching this point means the
                    // provider has already accepted (in-flight) or settled the
                    // payment, so the money is committed. A customer who finished
                    // the hosted flow just after the session's TTL must still get
                    // their subscription — erroring would only strand a paid
                    // customer (and, since CheckoutError isn't terminal in the
                    // webhook handler, retry-storm to dead-letter). Log and proceed.
                    if session.is_expired() {
                        log::warn!(
                            "Materializing checkout session {} after its expiry ({:?} phase): payment already committed",
                            checkout_session_id,
                            phase
                        );
                    }

                    let charge_result = DirectChargeResult {
                        payment_intent: crate::domain::payment_transactions::PaymentIntent {
                            external_id: event.provider_transaction_id.clone().unwrap_or_default(),
                            transaction_id: event.payment_transaction_id,
                            tenant_id: event.tenant_id,
                            amount_requested: event.amount,
                            amount_received: Some(event.amount),
                            currency: event.currency.clone(),
                            next_action: None,
                            status: crate::domain::PaymentStatusEnum::Settled,
                            last_payment_error: None,
                            processed_at: event.processed_at,
                        },
                        transaction_id: event.payment_transaction_id,
                        amount: event.amount,
                        currency: event.currency.clone(),
                        payment_method_id: event.payment_method_id.ok_or_else(|| {
                            Report::new(StoreError::InvalidArgument(
                                "Payment method ID required".to_string(),
                            ))
                        })?,
                    };

                    // Billing mode for the subscription-creating arms (SelfServe,
                    // SubscriptionActivation, PlanChange free-trial). Settled →
                    // record Paid; AcceptedInFlight → finalize Processing and link
                    // the still-Pending tx. Built fresh per use (mode isn't Clone).
                    let make_billing_mode = || match phase {
                        CheckoutPaymentPhase::Settled => InvoiceBillingMode::AlreadyPaid {
                            charge_result: charge_result.clone(),
                            existing_transaction_id: Some(event.payment_transaction_id),
                        },
                        CheckoutPaymentPhase::AcceptedInFlight => {
                            InvoiceBillingMode::AcceptedInFlight {
                                existing_transaction_id: event.payment_transaction_id,
                                amount: event.amount,
                                currency: event.currency.clone(),
                            }
                        }
                    };

                    let subscription_id = match session.checkout_type {
                        CheckoutType::SelfServe => {
                            // Create subscription now that payment is confirmed
                            let coupon_ids = self
                                .resolve_coupon_ids_for_checkout_tx(
                                    conn,
                                    event.tenant_id,
                                    &session,
                                    None,
                                )
                                .await?;

                            let start_date = session.billing_start_date.unwrap_or(priced_on);

                            let create_subscription =
                                session.to_create_subscription(start_date, coupon_ids);

                            let context = self
                                .gather_subscription_context(
                                    conn,
                                    std::slice::from_ref(&create_subscription),
                                    event.tenant_id,
                                    &self.store.settings.crypt_key,
                                )
                                .await?;

                            let detailed_subscriptions =
                                self.build_subscription_details(&[create_subscription], &context)?;

                            let detailed_sub = detailed_subscriptions.into_iter().next().ok_or(
                                Report::new(StoreError::InsertError)
                                    .attach("No subscription details built"),
                            )?;

                            // Payment already happened, so no provider setup runs — but the row
                            // must still be inserted `checkout: true` (like the sync self-serve
                            // path): a pending-activation sub with `current_period_end = None`
                            // AND `pending_checkout = false` reads as COMPLETED to the invoice
                            // line computation, which then bills nothing and the whole
                            // materialization fails. `activate_subscription_after_payment`
                            // below clears the flag and sets the real period.
                            let payment_result = crate::services::PaymentSetupResult { checkout: true };

                            let processed = self.process_subscription(
                                &detailed_sub,
                                &payment_result,
                                &context,
                                event.tenant_id,
                                None,
                            )?;

                            // Skip coupon validation since coupons were already validated before
                            // charging. The customer already paid the discounted price.
                            let created_subscriptions = self
                                .persist_subscriptions_skip_coupon_validation(
                                    conn,
                                    &[processed],
                                    event.tenant_id,
                                    &self.store.settings.jwt_secret,
                                    &self.store.settings.public_url,
                                )
                                .await?;

                            let created_subscription =
                                created_subscriptions.into_iter().next().ok_or(
                                    Report::new(StoreError::InsertError)
                                        .attach("No subscription created"),
                                )?;

                            self.bill_subscription_tx(
                                conn,
                                event.tenant_id,
                                created_subscription.id,
                                make_billing_mode(),
                            )
                            .await?
                            .ok_or(
                                Report::new(StoreError::InsertError)
                                    .attach("Failed to create invoice for subscription"),
                            )?;

                            // Activate the subscription now that payment is confirmed
                            let billing_start_date = session.billing_start_date.unwrap_or(priced_on);

                            // Get subscription to determine the period
                            let subscription = SubscriptionRow::get_subscription_by_id(
                                conn,
                                &event.tenant_id,
                                created_subscription.id,
                            )
                            .await?;

                            let billing_day_anchor = session
                                .billing_day_anchor
                                .map(|a| a as u32)
                                .unwrap_or_else(|| billing_start_date.day());

                            let is_paid_trial_flag = is_paid_trial(
                                conn,
                                subscription.subscription.plan_version_id,
                                event.tenant_id,
                                session.trial_duration_days.is_some(),
                            )
                            .await?;

                            self.activate_subscription_after_payment(
                                conn,
                                &created_subscription.id,
                                &event.tenant_id,
                                PaymentActivationParams {
                                    billing_start_date,
                                    trial_duration: session.trial_duration_days,
                                    is_paid_trial: is_paid_trial_flag,
                                    billing_day_anchor,
                                    period: subscription.subscription.period.into(),
                                },
                            )
                            .await?;

                            created_subscription.id
                        }
                        CheckoutType::SubscriptionActivation => {
                            let subscription_id = session.subscription_id.ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "SubscriptionActivation checkout missing subscription_id"
                                        .to_string(),
                                ))
                            })?;

                            let subscription = self
                                .store
                                .get_subscription_details_with_conn(
                                    conn,
                                    event.tenant_id,
                                    subscription_id,
                                )
                                .await?;

                            let customer: crate::domain::Customer = CustomerRow::find_by_id(
                                conn,
                                &subscription.subscription.customer_id,
                                &event.tenant_id,
                            )
                            .await
                            .map_err(Into::into)
                            .and_then(TryInto::try_into)?;

                            self.bill_subscription_with_data_tx(
                                conn,
                                event.tenant_id,
                                subscription.clone(),
                                customer,
                                make_billing_mode(),
                            )
                            .await?;

                            // Activate the subscription if it was pending checkout
                            let should_activate = subscription.subscription.activated_at.is_none()
                                && subscription.subscription.activation_condition
                                    == crate::domain::enums::SubscriptionActivationCondition::OnCheckout;

                            if should_activate {
                                let billing_start_date = subscription
                                    .subscription
                                    .billing_start_date
                                    .unwrap_or_else(|| Utc::now().date_naive());

                                let is_paid_trial_flag = is_paid_trial(
                                    conn,
                                    subscription.subscription.plan_version_id,
                                    event.tenant_id,
                                    subscription.subscription.trial_duration.is_some(),
                                )
                                .await?;

                                self.activate_subscription_after_payment(
                                    conn,
                                    &subscription_id,
                                    &event.tenant_id,
                                    PaymentActivationParams {
                                        billing_start_date,
                                        trial_duration: subscription
                                            .subscription
                                            .trial_duration
                                            .map(|d| d as i32),
                                        is_paid_trial: is_paid_trial_flag,
                                        billing_day_anchor: subscription.subscription.billing_day_anchor
                                            as u32,
                                        period: subscription.subscription.period,
                                    },
                                )
                                .await?;
                            }

                            subscription_id
                        }
                        CheckoutType::PlanChange => {
                            let subscription_id = session.subscription_id.ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "PlanChange checkout missing subscription_id".to_string(),
                                ))
                            })?;

                            let new_plan_version_id = session.plan_version_id;
                            // The checkout preview and the synchronous confirm both price the
                            // change at the session's change_date, so rebuild at that date while
                            // it is still inside the current period; only a period rollover
                            // (async payments can take days) forces today, since the original
                            // date is then invalid.
                            let today = {
                                let current_period_start = SubscriptionRow::get_subscription_by_id(
                                    conn,
                                    &event.tenant_id,
                                    subscription_id,
                                )
                                .await?
                                .subscription
                                .current_period_start;
                                match session.change_date {
                                    Some(d) if d >= current_period_start => d,
                                    other => {
                                        let now = Utc::now().date_naive();
                                        log::warn!(
                                            "Plan-change checkout {} priced at {other:?} but the period rolled over (starts {current_period_start}); applying at {now}",
                                            checkout_session_id
                                        );
                                        now
                                    }
                                }
                            };

                            let prepared = self
                                .prepare_plan_change_tx(
                                    conn,
                                    subscription_id,
                                    event.tenant_id,
                                    new_plan_version_id,
                                    &[],
                                    today,
                                )
                                .await?;

                            let is_free_trial = prepared.is_free_trial();

                            // Apply the plan change (handles trial→Active transition)
                            self.execute_plan_change_tx(
                                conn,
                                &prepared,
                                subscription_id,
                                event.tenant_id,
                                new_plan_version_id,
                                today,
                            )
                            .await?;

                            if is_free_trial {
                                // Create first invoice linked to the payment.
                                self.bill_subscription_tx(
                                    conn,
                                    event.tenant_id,
                                    subscription_id,
                                    make_billing_mode(),
                                )
                                .await?;
                            } else {
                                // Normal plan change: create adjustment invoice
                                let net_amount = prepared.proration.net_amount_cents;
                                if net_amount != 0 {
                                    let invoice = self
                                        .create_adjustment_invoice(
                                            conn,
                                            event.tenant_id,
                                            &prepared.subscription_details.subscription,
                                            &prepared.subscription_details.customer,
                                            &prepared.proration,
                                        )
                                        .await?;

                                    if let Some(inv) = &invoice {
                                        self.finalize_invoice_tx(conn, &Actor::System, inv.id, event.tenant_id, false, &None,
                                        )
                                        .await?;

                                        self.settle_or_link_adjustment_invoice(
                                            conn,
                                            event.tenant_id,
                                            inv.id,
                                            phase,
                                            event.payment_transaction_id,
                                            charge_result.amount,
                                            &charge_result.currency,
                                            charge_result.payment_intent.processed_at,
                                        )
                                        .await?;
                                    }
                                }
                            }

                            subscription_id
                        }
                        CheckoutType::AddonPurchase => {
                            let subscription_id = session.subscription_id.ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "AddonPurchase checkout missing subscription_id".to_string(),
                                ))
                            })?;

                            let create_add_ons = session.add_ons.as_ref().ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "AddonPurchase checkout session has no add_ons".to_string(),
                                ))
                            })?;

                            let addon_ids: Vec<_> =
                                create_add_ons.add_ons.iter().map(|a| a.add_on_id).collect();
                            let addons = {
                                let rows = diesel_models::add_ons::AddOnRow::list_by_ids(
                                    conn,
                                    &addon_ids,
                                    &event.tenant_id,
                                )
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                                crate::repositories::add_ons::enrich_add_ons(
                                    conn,
                                    rows,
                                    event.tenant_id,
                                )
                                .await?
                            };

                            let product_ids: Vec<_> =
                                addons.iter().map(|a| a.product_id).collect();
                            let price_ids: Vec<_> = addons.iter().map(|a| a.price_id).collect();
                            let (prices_by_id, products_by_id) =
                                crate::repositories::subscriptions::fetch_prices_and_products(
                                    conn,
                                    event.tenant_id,
                                    price_ids.into_iter(),
                                    product_ids.into_iter(),
                                )
                                .await?;

                            let addon_effective_from = session.change_date.unwrap_or(priced_on);

                            crate::repositories::subscription_add_ons::resolve_and_insert_checkout_addons(
                                conn,
                                subscription_id,
                                &addons,
                                &create_add_ons.add_ons,
                                &products_by_id,
                                &prices_by_id,
                                addon_effective_from,
                            )
                            .await?;

                            // Create prorated one-off invoice for the addon purchase
                            let result = self
                                .compute_addon_purchase_invoice(
                                    conn,
                                    event.tenant_id,
                                    subscription_id,
                                    &create_add_ons.add_ons,
                                    &addons,
                                    &products_by_id,
                                    &prices_by_id,
                                    priced_on,
                                )
                                .await?;

                            let content =
                                crate::services::invoices::AdjustmentInvoiceContent {
                                    computed: result.invoice_content,
                                    invoicing_entity: None,
                                };

                            let draft = self
                                .create_adjustment_invoice_from_content(
                                    conn,
                                    &result.subscription,
                                    &result.customer,
                                    &result.proration,
                                    content,
                                )
                                .await?;

                            if let Some(invoice) = draft {
                                self.finalize_invoice_tx(conn, &Actor::System, invoice.id, event.tenant_id,
                                    false,
                                    &None,
                                )
                                .await?;

                                self.settle_or_link_adjustment_invoice(
                                    conn,
                                    event.tenant_id,
                                    invoice.id,
                                    phase,
                                    event.payment_transaction_id,
                                    charge_result.amount,
                                    &charge_result.currency,
                                    charge_result.payment_intent.processed_at,
                                )
                                .await?;
                            }

                            subscription_id
                        }
                    };

                    CheckoutSessionRow::mark_completed(
                        conn,
                        event.tenant_id,
                        checkout_session_id,
                        subscription_id,
                        chrono::Utc::now(),
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    log::info!(
                        "Completed checkout session {} with subscription {} after async payment",
                        checkout_session_id,
                        subscription_id
                    );

                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        Ok(())
    }

    /// Entry point from the GoCardless `billing_requests.fulfilled` webhook for a
    /// combined mandate+payment CHECKOUT Billing Request. The mandate is already
    /// attached (as a payment method) by the webhook handler; here we bind the
    /// provider-created payment to the pre-created Pending checkout transaction and
    /// materialize the subscription+invoice in-flight.
    ///
    /// Idempotent under webhook redelivery: once the tx is linked to an invoice
    /// (materialized) or no live checkout tx remains, it is a no-op.
    pub async fn on_hosted_checkout_fulfilled(
        &self,
        tenant_id: common_domain::ids::TenantId,
        checkout_session_id: common_domain::ids::CheckoutSessionId,
        payment_method_id: common_domain::ids::CustomerPaymentMethodId,
        provider_payment_id: Option<String>,
        processed_at: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<()> {
        use diesel_models::enums::PaymentStatusEnum as DbPaymentStatus;
        use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};

        // Bind the created payment + method to the checkout tx and read back a
        // settlement-style event. We look the tx up at ANY status (not just
        // Pending/Ready): if `payments.confirmed` raced ahead of this fulfilled
        // event it will already have driven the tx to Settled — we must still
        // materialize, just as Paid instead of Processing. A separate short tx
        // from the materialization is fine: both halves are idempotent.
        //
        // The bool is `settled`: true → materialize Paid (settled phase);
        // false → materialize Processing (accepted-in-flight).
        let outcome = self
            .store
            .transaction(|conn| {
                async move {
                    let Some(row) = PaymentTransactionRow::get_latest_by_checkout_session_id(
                        conn,
                        checkout_session_id,
                        tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    else {
                        // No checkout tx at all: a mandate-only (non-checkout) BR,
                        // or the tx was reaped. Nothing to materialize.
                        log::info!(
                            "hosted checkout fulfilled for session {checkout_session_id}: no checkout transaction found; ignoring"
                        );
                        return Ok::<Option<(PaymentTransactionEvent, bool)>, Report<StoreError>>(
                            None,
                        );
                    };

                    // Already materialized (linked to its invoice) → idempotent no-op.
                    if row.invoice_id.is_some() {
                        log::info!(
                            "hosted checkout fulfilled for session {checkout_session_id}: transaction {} already materialized (invoice {:?})",
                            row.id,
                            row.invoice_id
                        );
                        return Ok(None);
                    }

                    let settled = match row.status {
                        DbPaymentStatus::Pending | DbPaymentStatus::Ready => false,
                        DbPaymentStatus::Settled => true,
                        // Failed/Cancelled/Refunded before fulfillment: no funds to
                        // activate against. Don't materialize; leave it for dunning /
                        // manual review rather than creating an unpaid subscription.
                        terminal => {
                            log::warn!(
                                "hosted checkout fulfilled for session {checkout_session_id}: transaction {} is {:?}, not materializing",
                                row.id,
                                terminal
                            );
                            return Ok(None);
                        }
                    };

                    let patched = PaymentTransactionRowPatch {
                        id: row.id,
                        // Don't clobber a provider id already set by an earlier
                        // payments.* event; only fill it in when we have one.
                        provider_transaction_id: provider_payment_id.clone().map(Some),
                        payment_method_id: Some(Some(payment_method_id)),
                        // Redirect consumed — the customer is back.
                        next_action: Some(None),
                        ..Default::default()
                    }
                    .patch(conn, tenant_id, row.id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let event = PaymentTransactionEvent {
                        id: common_domain::ids::EventId::new(),
                        payment_transaction_id: patched.id,
                        tenant_id,
                        invoice_id: None,
                        checkout_session_id: Some(checkout_session_id),
                        provider_transaction_id: patched.provider_transaction_id.clone(),
                        // Settled: use when the tx actually settled; in-flight: None.
                        processed_at: if settled { patched.processed_at } else { processed_at },
                        refunded_at: None,
                        amount: patched.amount,
                        currency: patched.currency,
                        payment_method_id: Some(payment_method_id),
                        status: if settled {
                            crate::domain::PaymentStatusEnum::Settled
                        } else {
                            crate::domain::PaymentStatusEnum::Pending
                        },
                        payment_type: crate::domain::PaymentTypeEnum::Payment,
                        error_type: None,
                        receipt_pdf_id: None,
                        pending_plan_version_id: patched.pending_plan_version_id,
                        amount_refunded: 0,
                    };

                    Ok(Some((event, settled)))
                }
                .scope_boxed()
            })
            .await?;

        let Some((event, settled)) = outcome else {
            return Ok(());
        };

        if settled {
            // payments.confirmed won the race: materialize straight to Paid.
            self.on_checkout_payment_settled(event, checkout_session_id)
                .await
        } else {
            self.on_checkout_payment_accepted_in_flight(event, checkout_session_id)
                .await
        }
    }

    /// Record a checkout payment against a freshly finalized adjustment invoice
    /// (plan-change proration / addon), phase-appropriately. Always links the tx
    /// to the invoice so an in-flight payment's later settlement routes through
    /// `on_invoice_payment_settled`; then Settled → apply the amount + mark Paid,
    /// AcceptedInFlight → mark Processing and leave `amount_due` for settlement.
    #[allow(clippy::too_many_arguments)]
    async fn settle_or_link_adjustment_invoice(
        &self,
        conn: &mut crate::store::PgConn,
        tenant_id: common_domain::ids::TenantId,
        invoice_id: common_domain::ids::InvoiceId,
        phase: CheckoutPaymentPhase,
        transaction_id: common_domain::ids::PaymentTransactionId,
        amount: i64,
        currency: &str,
        processed_at: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<()> {
        // In-flight, the adjustment invoice was recomputed at fulfillment time
        // (with `today`), so it can drift from the amount the provider actually
        // froze and is collecting. Guard against silently mis-collecting — the
        // same ±1 currency/amount check the subscription-creating arms apply in
        // `bill.rs` (`AcceptedInFlight`). A mismatch surfaces (CheckoutError →
        // pgmq dead-letter) rather than paying/overcharging against a wrong total.
        // The Settled phase already reconciles via `recompute_amount_due` on the
        // settlement webhook, so it needs no pre-check here.
        if matches!(phase, CheckoutPaymentPhase::AcceptedInFlight) {
            let invoice = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            if !invoice.currency.eq_ignore_ascii_case(currency) {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "in-flight adjustment currency mismatch: invoice {} vs payment {}",
                    invoice.currency, currency
                )));
            }
            if (invoice.amount_due - amount).abs() > 1 {
                return Err(Report::new(StoreError::CheckoutError).attach(format!(
                    "in-flight adjustment amount drift for invoice {}: invoice_due {} vs frozen charge {}",
                    invoice_id, invoice.amount_due, amount
                )));
            }
        }

        self.link_transaction_to_invoice(conn, tenant_id, transaction_id, invoice_id)
            .await?;

        match phase {
            CheckoutPaymentPhase::Settled => {
                InvoiceRow::apply_transaction(conn, invoice_id, tenant_id, amount).await?;
                InvoiceRow::apply_payment_status(
                    conn,
                    invoice_id,
                    tenant_id,
                    diesel_models::enums::InvoicePaymentStatus::Paid,
                    processed_at,
                )
                .await?;
            }
            CheckoutPaymentPhase::AcceptedInFlight => {
                InvoiceRow::apply_payment_status(
                    conn,
                    invoice_id,
                    tenant_id,
                    diesel_models::enums::InvoicePaymentStatus::Processing,
                    None,
                )
                .await?;
            }
        }

        Ok(())
    }
}
