use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use crate::domain::{Invoice, InvoicePaymentStatus, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::services::Services;
use crate::services::credit::ReissuedPurchase;
use crate::services::credit::collected_payment::keep_collected_payment_tx;
use crate::services::payment::hosted_setup::DefaultMethodAdoption;
use crate::services::payment_reminders::sweep::ReopenedReminders;
use crate::services::subscriptions::PaymentActivationParams;
use crate::services::subscriptions::start::{StartTransition, mark_subscription_started_at_tx};
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
        let is_top_up = matches!(
            event.payment_type,
            crate::domain::PaymentTypeEnum::TopUp | crate::domain::PaymentTypeEnum::AutoTopup
        );
        // The event can predate a failed attempt discarding the document: the payment row is current.
        let current_invoice = if is_top_up {
            let mut conn = self.store.get_conn().await?;
            PaymentTransactionRow::get_by_id(
                &mut conn,
                event.payment_transaction_id,
                event.tenant_id,
            )
            .await?
            .invoice_id
        } else {
            event.invoice_id
        };
        // A top-up captured after its failed attempt discarded the document gets it reissued.
        let event_invoice = match (current_invoice, event.checkout_session_id) {
            (None, _) if is_top_up => match self.reissue_captured_purchase(&event).await? {
                Some(ReissuedPurchase::Document(invoice_id)) => Some(invoice_id),
                Some(ReissuedPurchase::Detached) => {
                    return self
                        .refund_detached_capture(event.tenant_id, event.payment_transaction_id)
                        .await;
                }
                None => None,
            },
            (invoice_id, _) => invoice_id,
        };
        let settled = (
            event.tenant_id,
            event.payment_transaction_id,
            event.payment_method_id,
        );
        match (event_invoice, event.checkout_session_id) {
            (Some(invoice_id), _) => self.on_invoice_payment_settled(event, invoice_id).await?,
            (None, Some(checkout_session_id)) => {
                self.on_checkout_payment_settled(event, checkout_session_id)
                    .await?
            }
            (None, None) => {
                log::warn!(
                    "Payment transaction {} has neither invoice_id nor checkout_session_id",
                    event.payment_transaction_id
                );
                return Err(Report::new(StoreError::InvalidArgument(
                    "Payment transaction must have either invoice_id or checkout_session_id"
                        .to_string(),
                )));
            }
        }
        if let (tenant_id, payment_id, Some(payment_method_id)) = settled {
            self.adopt_paid_payment_method(tenant_id, payment_id, payment_method_id)
                .await;
        }
        Ok(())
    }

    /// The card a subscription checkout settled with becomes the default (its renewals bill it);
    /// another becomes it when saved for that payment, on-session. Best effort: the settlement stands.
    async fn adopt_paid_payment_method(
        &self,
        tenant_id: common_domain::ids::TenantId,
        payment_id: common_domain::ids::PaymentTransactionId,
        payment_method_id: common_domain::ids::CustomerPaymentMethodId,
    ) {
        let adopted = async {
            let (payment, subscription_checkout) = {
                let mut conn = self.store.get_conn().await?;
                let payment =
                    PaymentTransactionRow::get_by_id(&mut conn, payment_id, tenant_id).await?;
                let subscription_checkout = match payment.checkout_session_id {
                    Some(session_id) => matches!(
                        diesel_models::checkout_sessions::CheckoutSessionRow::get_by_id(
                            &mut conn, tenant_id, session_id,
                        )
                        .await?
                        .checkout_type,
                        diesel_models::enums::CheckoutTypeEnum::SelfServe
                            | diesel_models::enums::CheckoutTypeEnum::SubscriptionActivation
                    ),
                    None => false,
                };
                (payment, subscription_checkout)
            };
            let method = self
                .store
                .get_payment_method_by_id(&tenant_id, &payment_method_id)
                .await?;
            // A top-up the customer makes in the portal is on-session; an automatic one is not.
            let on_session = match payment.payment_type {
                diesel_models::enums::PaymentTypeEnum::Payment => {
                    payment.initiated_by_customer_id.is_some()
                        || payment.checkout_session_id.is_some()
                }
                diesel_models::enums::PaymentTypeEnum::TopUp => {
                    payment.initiated_by_customer_id.is_some()
                }
                _ => false,
            };
            self.adopt_default_payment_method(
                Actor::System,
                tenant_id,
                method.customer_id,
                payment_method_id,
                if subscription_checkout {
                    DefaultMethodAdoption::Replace
                } else {
                    DefaultMethodAdoption::Paid {
                        on_session,
                        started_at: payment.created_at,
                    }
                },
            )
            .await
        }
        .await;
        if let Err(e) = adopted {
            log::warn!(
                "payment method {payment_method_id} not made the default after its payment settled: {e:?}"
            );
        }
    }

    /// Handle payment settlement for a standard invoice payment
    pub(in crate::services) async fn on_invoice_payment_settled(
        &self,
        event: PaymentTransactionEvent,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<()> {
        let event = &event;
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
            async move {
                services
                    .on_invoice_payment_settled_tx(conn, event, invoice_id, Utc::now())
                    .await
            }
            .scope_boxed()
        })
        .await
    }

    /// In the caller's transaction, with `now` the present the funded balance is judged at.
    /// Idempotent: a redelivered settlement recomputes the same balance and funds nothing again.
    pub(in crate::services) async fn on_invoice_payment_settled_tx(
        &self,
        conn: &mut crate::store::PgConn,
        event: &PaymentTransactionEvent,
        invoice_id: common_domain::ids::InvoiceId,
        now: chrono::DateTime<Utc>,
    ) -> StoreResult<()> {
        // Boxed so callers hold a pointer, not this state machine; see services/mod.rs.
        Box::pin(async move {
            let invoice = InvoiceRow::select_for_update_by_id(conn, event.tenant_id, invoice_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let lock = &invoice.lock;
            // A redelivered settlement finds the invoice paid: what its first delivery did stands.
            let was_paid = invoice.invoice.payment_status
                == diesel_models::enums::InvoicePaymentStatus::Paid;

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
                self.finalize_invoice_tx(
                    conn,
                    &actor,
                    invoice.invoice.id,
                    invoice.invoice.tenant_id,
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

            let collected = crate::repositories::credit::customer_credit::PaymentMovement::collection(
                event.payment_transaction_id,
                collected_at(event),
            );
            crate::services::credit::collected_payment::reconcile_overpayment_tx(
                conn,
                &*self.store.hooks.credit,
                event.tenant_id,
                invoice_id,
                Some(crate::services::credit::collected_payment::Collection {
                    payment: event.payment_transaction_id,
                    at: collected_at(event),
                }),
                crate::domain::wallet_events::CreditUnrecoveredCause::PaymentReversed,
                Some(&collected),
            )
            .await?;

            // Money paid into a reopened purchase restores, in proportion, what a reversal took.
            crate::services::credit::reconcile_purchased_credit_tx(
                conn,
                &*self.store.hooks.credit,
                lock,
                invoice_id,
                Some(&collected),
            )
            .await?;

            let completed = res.amount_due == 0;

            if completed {
                let invoice: Invoice = res.try_into()?;

                if invoice.payment_status != InvoicePaymentStatus::Paid {
                    self.apply_invoice_paid_at_tx(
                        conn,
                        event.tenant_id,
                        invoice_id,
                        event.processed_at,
                        now,
                    )
                    .await?;

                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            event.tenant_id,
                            &actor,
                            vec![OutboxEvent::invoice_paid((&invoice).into())],
                        )
                        .await?;
                }

                // The top-up this payment bought settles once its credit is funded.
                crate::services::credit::complete_purchase_tx(
                    conn,
                    &*self.store.hooks.credit,
                    lock,
                    event.payment_transaction_id,
                    now,
                )
                .await?;

                // Apply deferred plan change if the transaction carries a pending plan version.
                if !was_paid
                    && let (Some(sub_id), Some(target_pvid)) =
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
                    // A savepoint: a change that can no longer apply never rolls the payment back;
                    // the invoice it paid is credited to the customer's balance instead.
                    let legacy = &invoice;
                    let applied = self
                        .store
                        .transaction_with(conn, |conn| {
                            async move {
                                let mut prepared = self
                                    .prepare_plan_change_tx(
                                        conn,
                                        sub_id,
                                        event.tenant_id,
                                        target_pvid,
                                        &[],
                                        change_date,
                                    )
                                    .await?;
                                // Only an earlier release links a pending plan change to its
                                // adjustment invoice: that invoice already netted the old plan.
                                prepared.adopt_legacy_adjustment(legacy);
                                self.execute_plan_change_tx(
                                    conn,
                                    &Actor::System,
                                    &prepared,
                                    sub_id,
                                    event.tenant_id,
                                    target_pvid,
                                    change_date,
                                )
                                .await
                                .map(|_| ())
                            }
                            .scope_boxed()
                        })
                        .await;
                    if let Err(error) = applied {
                        if !can_never_apply(&error) {
                            return Err(error);
                        }
                        log::error!(
                            "Deferred plan change of subscription {sub_id} could not apply after its payment: {error:?}"
                        );
                        let note = crate::repositories::credit_notes::create_user_credit_note_tx(
                            &self.store,
                            conn,
                            event.tenant_id,
                            &Actor::System,
                            crate::repositories::credit_notes::CreateCreditNoteParams {
                                invoice_id: invoice.id,
                                line_items: vec![],
                                reason: Some("Plan change could not apply".to_owned()),
                                memo: None,
                                credit_type: crate::domain::enums::CreditType::CreditToBalance,
                                custom_properties: None,
                            },
                            false,
                            false,
                        )
                        .await?;
                        crate::repositories::credit_notes::finalize_credit_note_tx(
                            &self.store,
                            conn,
                            event.tenant_id,
                            &Actor::System,
                            note.id,
                        )
                        .await?;
                    }
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
                        && !subscription.subscription.status.is_terminal()
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
                        mark_subscription_started_at_tx(
                            conn,
                            &*self.store.hooks.credit,
                            event.tenant_id,
                            *subscription_id,
                            StartTransition::Activate {
                                current_period_start,
                                current_period_end,
                                next_cycle_action,
                                cycle_index,
                                status: status.into(),
                            },
                            now,
                        )
                        .await?;
                        diesel_models::query::credit_inputs::bump_customer(conn, lock).await?;
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
        })
        .await
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
        // Credit kept from the payment goes first; the invoice it paid reopens for the rest.
        self.store
            .transaction(|conn| {
                async move {
                    crate::services::credit::collected_payment::follow_collected_payment_tx(
                        conn,
                        &*self.store.hooks.credit,
                        event.tenant_id,
                        event.payment_transaction_id,
                        None,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await?;
        let Some(invoice_id) = event.invoice_id else {
            log::warn!(
                "Reversed payment transaction {} has no invoice_id; only the credit kept from it \
                 follows the reversal",
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
                    crate::services::credit::collected_payment::reconcile_overpayment_tx(
                        conn,
                        &*self.store.hooks.credit,
                        event.tenant_id,
                        invoice_id,
                        None,
                        crate::domain::wallet_events::CreditUnrecoveredCause::PaymentReversed,
                        None,
                    )
                    .await?;

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
                    self.store
                        .internal
                        .on_invoice_reopened(
                            conn,
                            event.tenant_id,
                            invoice_id,
                            recomputed.payment_status == InvoicePaymentStatus::Paid,
                            true,
                            ReopenedReminders::after_reversal(event.reversal_kind),
                            chrono::Utc::now().naive_utc(),
                        )
                        .await?;

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
        use crate::domain::checkout_sessions::{CheckoutSessionStatus, CheckoutType};
        use crate::services::InvoiceBillingMode;
        use crate::services::checkout_completion::DirectChargeResult;
        use diesel_models::customers::CustomerRow;

        let event = &event;
        let mut conn = self.store.get_conn().await?;
        self.transaction_reading_usage_first(&mut conn, |services, conn| {
                async move {
                    // Locked (behind its customer): this is the single serialization point for
                    // materializing a checkout. Two concurrent deliveries (a
                    // redelivered `billing_requests.fulfilled`, or fulfilled racing
                    // the confirmed-won-race Settled path) would otherwise both read
                    // `is_completed() == false` and each create a subscription +
                    // invoice from one checkout. The lock makes the second block,
                    // then observe Completed below and no-op.
                    let (lock, session_row) = CheckoutSessionRow::get_by_id_for_update(
                        conn,
                        event.tenant_id,
                        checkout_session_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    let session: crate::domain::checkout_sessions::CheckoutSession =
                        session_row.into();

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

                    // A plan change given up before its payment settled is never applied late:
                    // another change may have followed. The money is kept as credit.
                    if session.checkout_type == CheckoutType::PlanChange
                        && matches!(
                            session.status,
                            CheckoutSessionStatus::Cancelled | CheckoutSessionStatus::Expired
                        )
                    {
                        if matches!(phase, CheckoutPaymentPhase::Settled) {
                            keep_collected_payment_tx(
                                conn,
                                &*services.store.hooks.credit,
                                event.tenant_id,
                                session.customer_id,
                                event.payment_transaction_id,
                                None,
                                event.amount,
                                &event.currency,
                                collected_at(event),
                                "Payment for a plan change checkout that was cancelled or expired",
                            )
                            .await?;
                        }
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
                            decline_kind: None,
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
                        CheckoutType::TopUp => return Err(crate::services::credit::precondition("top-up payment must settle its purchase invoice")),
                        CheckoutType::SelfServe => {
                            // Create subscription now that payment is confirmed
                            let coupon_ids = services
                                .paid_checkout_coupon_ids_tx(conn, event.tenant_id, &session)
                                .await?;

                            let start_date = session.billing_start_date.unwrap_or(priced_on);

                            let create_subscription =
                                session.to_create_subscription(start_date, coupon_ids)?;

                            let context = services
                                .gather_subscription_context(
                                    conn,
                                    std::slice::from_ref(&create_subscription),
                                    event.tenant_id,
                                    &services.store.settings.crypt_key,
                                )
                                .await?;

                            let detailed_subscriptions =
                                services.build_subscription_details(&[create_subscription], &context)?;

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

                            let processed = services.process_subscription(
                                &detailed_sub,
                                &payment_result,
                                &context,
                                event.tenant_id,
                                None,
                            )?;

                            // Skip coupon validation since coupons were already validated before
                            // charging. The customer already paid the discounted price.
                            let created_subscriptions = services
                                .persist_subscriptions_skip_coupon_validation(
                                    conn,
                                    &Actor::Customer {
                                        id: session.customer_id,
                                    },
                                    &[processed],
                                    event.tenant_id,
                                    &services.store.settings.jwt_secret,
                                    &services.store.settings.public_url,
                                )
                                .await?;

                            let created_subscription =
                                created_subscriptions.into_iter().next().ok_or(
                                    Report::new(StoreError::InsertError)
                                        .attach("No subscription created"),
                                )?;

                            services.bill_subscription_tx(
                                conn,
                                &Actor::Customer {
                                    id: session.customer_id,
                                },
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

                            services.activate_subscription_after_payment(
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

                            // Bill + activate only if still activatable: a subscription aborted or
                            // cancelled (or already activated) between checkout creation and a late
                            // settlement must not be charged again or resurrected.
                            let should_activate = subscription.subscription.activated_at.is_none()
                                && !subscription.subscription.status.is_terminal()
                                && subscription.subscription.activation_condition
                                    == crate::domain::enums::SubscriptionActivationCondition::OnCheckout;

                            if should_activate {
                                services.bill_subscription_with_data_tx(
                                    conn,
                                    &Actor::Customer {
                                        id: session.customer_id,
                                    },
                                    event.tenant_id,
                                    subscription.clone(),
                                    customer,
                                    make_billing_mode(),
                                    crate::services::invoices::PeriodBilling::Bill,
                                )
                                .await?;

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

                                services.activate_subscription_after_payment(
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
                            } else {
                                log::warn!(
                                    "Ignoring activation checkout settlement for subscription {}: not activatable (status {:?}, activated_at {:?})",
                                    subscription_id,
                                    subscription.subscription.status,
                                    subscription.subscription.activated_at
                                );
                            }

                            subscription_id
                        }
                        CheckoutType::PlanChange => {
                            let subscription_id = session.subscription_id.ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "PlanChange checkout missing subscription_id".to_string(),
                                ))
                            })?;

                            let new_plan_version_id = session.subscription_plan_version_id()?;
                            // The checkout preview and the synchronous confirm both price the
                            // change at the session's change_date, so rebuild at that date while
                            // it is still inside the current period; only a period rollover
                            // (async payments can take days) forces today, since the original
                            // date is then invalid.
                            let current_period_start = SubscriptionRow::get_subscription_by_id(
                                conn,
                                &event.tenant_id,
                                subscription_id,
                            )
                            .await?
                            .subscription
                            .current_period_start;
                            // Priced at the quoted date while it is still in the current period;
                            // after a rollover, today. The payment is then recorded for what it
                            // pays: the invoice is Paid only when nothing is left due.
                            let today = match session.change_date {
                                Some(d) if d >= current_period_start => d,
                                other => {
                                    let now = Utc::now().date_naive();
                                    log::warn!(
                                        "Plan-change checkout {} priced at {other:?} but the period rolled over (starts {current_period_start}); applying at {now}",
                                        checkout_session_id
                                    );
                                    now
                                }
                            };

                            // Applied in a savepoint: a payment collected is never rolled back
                            // by a change that can no longer apply; it is kept as credit instead.
                            // None: a free trial, billed by its first invoice; Some(None): the
                            // change billed nothing.
                            let actor = Actor::Customer {
                                id: session.customer_id,
                            };
                            let applied = {
                                let (actor, event, make_billing_mode) =
                                    (&actor, &event, &make_billing_mode);
                                services.store
                                    .transaction_with(conn, |conn| {
                                        async move {
                                            let prepared = services
                                                .prepare_plan_change_tx(
                                                    conn,
                                                    subscription_id,
                                                    event.tenant_id,
                                                    new_plan_version_id,
                                                    &[],
                                                    today,
                                                )
                                                .await?;
                                            if prepared.is_free_trial() {
                                                services.execute_plan_change_tx(
                                                    conn,
                                                    actor,
                                                    &prepared,
                                                    subscription_id,
                                                    event.tenant_id,
                                                    new_plan_version_id,
                                                    today,
                                                )
                                                .await?;
                                                services.bill_subscription_tx(
                                                    conn,
                                                    actor,
                                                    event.tenant_id,
                                                    subscription_id,
                                                    make_billing_mode(),
                                                )
                                                .await?;
                                                return Ok::<_, Report<StoreError>>(None);
                                            }
                                            let (invoice, _) = services
                                                .apply_and_invoice_plan_change_tx(
                                                    conn,
                                                    actor,
                                                    &prepared,
                                                    new_plan_version_id,
                                                    today,
                                                )
                                                .await?;
                                            Ok(Some(invoice.map(|invoice| invoice.id)))
                                        }
                                        .scope_boxed()
                                    })
                                    .await
                            };
                            match applied {
                                Ok(Some(Some(invoice_id))) => {
                                    services.settle_or_link_adjustment_invoice(
                                        conn,
                                        event.tenant_id,
                                        invoice_id,
                                        phase,
                                        event.payment_transaction_id,
                                        &charge_result.currency,
                                        charge_result.payment_intent.processed_at,
                                    )
                                    .await?;
                                }
                                Ok(Some(None)) => {
                                    if matches!(phase, CheckoutPaymentPhase::Settled) {
                                        keep_collected_payment_tx(
                                            conn,
                                            &*services.store.hooks.credit,
                                            event.tenant_id,
                                            session.customer_id,
                                            event.payment_transaction_id,
                                            None,
                                            event.amount,
                                            &event.currency,
                                            collected_at(event),
                                            "Payment for a plan change that billed nothing",
                                        )
                                        .await?;
                                    }
                                }
                                Ok(None) => {}
                                Err(error) if can_never_apply(&error) => {
                                    log::error!(
                                        "Plan-change checkout {checkout_session_id} could not apply after its payment: {error:?}"
                                    );
                                    // Collected only once settled: an accepted payment waits.
                                    if matches!(phase, CheckoutPaymentPhase::AcceptedInFlight) {
                                        return Ok(());
                                    }
                                    keep_collected_payment_tx(
                                        conn,
                                        &*services.store.hooks.credit,
                                        event.tenant_id,
                                        session.customer_id,
                                        event.payment_transaction_id,
                                        None,
                                        event.amount,
                                        &event.currency,
                                        collected_at(event),
                                        "Payment for a plan change that could not apply",
                                    )
                                    .await?;
                                }
                                Err(error) => return Err(error),
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

                            let units = crate::repositories::credit_units::load_add_on_credit_units(
                                conn,
                                event.tenant_id,
                                &prices_by_id,
                                &create_add_ons.add_ons,
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
                                &units,
                                addon_effective_from,
                            )
                            .await?;

                            // Create prorated one-off invoice for the addon purchase
                            let result = services
                                .compute_addon_purchase_invoice(
                                    conn,
                                    event.tenant_id,
                                    subscription_id,
                                    &create_add_ons.add_ons,
                                    &addons,
                                    &products_by_id,
                                    &prices_by_id,
                                    &units,
                                    priced_on,
                                )
                                .await?;

                            let content =
                                crate::services::invoices::AdjustmentInvoiceContent {
                                    computed: result.invoice_content,
                                    invoicing_entity: None,
                                };

                            let draft = services
                                .create_adjustment_invoice_from_content(
                                    conn,
                                    &result.subscription,
                                    &result.customer,
                                    &result.proration,
                                    content,
                                )
                                .await?;

                            if let Some(invoice) = draft {
                                services.finalize_invoice_tx(conn, &Actor::Customer { id: session.customer_id }, invoice.id, event.tenant_id,
                                    false,
                                    &None,
                                )
                                .await?;

                                services.settle_or_link_adjustment_invoice(
                                    conn,
                                    event.tenant_id,
                                    invoice.id,
                                    phase,
                                    event.payment_transaction_id,
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
                        amount_reversed: 0,
                        amount_disputed: 0,
                        amount_reversed_excess: 0,
                        reversal_kind: None,
                        parent_transaction_id: None,
                        credit_note_id: None,
                        refund_mode: None,
                        decline_kind: None,
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
            // payments.confirmed won the race → Paid. That only happens if GC echoed our
            // payment_request metadata as the payment's resource_metadata (the pre-created
            // tx has no provider id yet); otherwise reconciliation is the backstop.
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
        currency: &str,
        processed_at: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<()> {
        // The amount may differ from the invoice re-priced at settlement and is recorded as such;
        // a currency never may.
        let currency_due = InvoiceRow::find_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .currency;
        if !currency_due.eq_ignore_ascii_case(currency) {
            return Err(Report::new(StoreError::CheckoutError).attach(format!(
                "adjustment currency mismatch: invoice {currency_due} vs payment {currency}"
            )));
        }
        self.link_transaction_to_invoice(conn, tenant_id, transaction_id, invoice_id)
            .await?;

        match phase {
            CheckoutPaymentPhase::Settled => {
                // Recorded for what it pays, recomputed from the settled payments: Paid only
                // when nothing is left due, any excess kept as customer credit.
                let invoice = InvoiceRow::recompute_amount_due_from_settled_payments(
                    conn, invoice_id, tenant_id,
                )
                .await?;
                if invoice.amount_due <= 0 {
                    self.apply_invoice_paid_tx(conn, tenant_id, invoice_id, processed_at)
                        .await?;
                } else {
                    InvoiceRow::apply_payment_status(
                        conn,
                        invoice_id,
                        tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::PartiallyPaid,
                        None,
                    )
                    .await?;
                }
                crate::services::credit::collected_payment::reconcile_overpayment_tx(
                    conn,
                    &*self.store.hooks.credit,
                    tenant_id,
                    invoice_id,
                    Some(crate::services::credit::collected_payment::Collection {
                        payment: transaction_id,
                        at: processed_at.map_or_else(Utc::now, |at| at.and_utc()),
                    }),
                    crate::domain::wallet_events::CreditUnrecoveredCause::PaymentReversed,
                    None,
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

/// A change that can never apply, however often retried: what its payment collected is kept as
/// credit. Anything else (database, metering, network) is retried with the event.
fn can_never_apply(error: &Report<StoreError>) -> bool {
    matches!(
        error.current_context(),
        StoreError::InvalidArgument(_)
            | StoreError::FailedPrecondition(_)
            | StoreError::InvalidPriceComponents(_)
            | StoreError::ValueNotFound(_)
            | StoreError::Forbidden(_)
    )
}

fn collected_at(event: &PaymentTransactionEvent) -> chrono::DateTime<Utc> {
    event.processed_at.map_or_else(Utc::now, |at| at.and_utc())
}
