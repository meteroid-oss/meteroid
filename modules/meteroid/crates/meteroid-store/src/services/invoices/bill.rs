use crate::StoreResult;
use crate::domain::entity_activity::Actor;
use crate::domain::payment_transactions::PaymentTransaction;
use crate::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use crate::domain::{Customer, DetailedInvoice, Invoice, PaymentStatusEnum, SubscriptionDetails};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::services::checkout_completion::DirectChargeResult;
use crate::store::PgConn;
use chrono::NaiveTime;
use common_domain::ids::{
    CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, SubscriptionId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::{ConnectorProviderEnum, PaymentMethodTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use error_stack::Report;

#[allow(clippy::large_enum_variant)]
pub enum InvoiceBillingMode {
    /// Post checkout. We request a payment and don't finalize the invoice until paid.
    FinalizeAfterPayment {
        payment_method_id: CustomerPaymentMethodId,
        total_amount_confirmation: u64,
        currency_confirmation: String,
    },
    /// Subscription renewal or terminated. If grace period, we schedule finalization. Else, we immediately finalize
    AwaitGracePeriodIfApplicable,
    /// Subscription created without checkout (ex: upgrade/downgrade). We immediately finalize
    Immediate,
    /// Payment was already collected before subscription creation
    AlreadyPaid {
        charge_result: DirectChargeResult,
        /// If set, update this existing transaction instead of creating a new one.
        /// Used for async payments where the transaction was created before the invoice.
        existing_transaction_id: Option<PaymentTransactionId>,
    },
    /// A hosted-checkout first payment that the provider has ACCEPTED but not yet
    /// settled: the mandate is signed and the debit submitted (GoCardless combined
    /// mandate+payment Billing Request), so the sale is contractual and the
    /// subscription activates now, but the funds land days later by webhook.
    /// Finalizes the invoice as `Processing` and links the pre-created Pending
    /// transaction; `amount_due` stays full until the settlement webhook flips it
    /// to `Paid` via `on_invoice_payment_settled`.
    AcceptedInFlight {
        existing_transaction_id: PaymentTransactionId,
        amount: i64,
        currency: String,
    },
}

/// When consolidation is enabled, the recurring renewal finalization is floored to at least this
/// many hours after the period boundary, so an aggressive (near-zero or negative) grace period
/// can't push the shared finalize deadline into the past before a customer's same-day sibling
/// drafts have been created (each is created lazily as its subscription's cycle is processed).
///
/// Best-effort, not a guarantee: same-day merging is reliable as long as the billing worker creates
/// all of a customer's due drafts within this window; a sibling drafted later still finalizes
/// standalone. The durable fix is a single per-(customer, date) finalization trigger.
const CONSOLIDATION_MIN_GRACE_HOURS: i32 = 1;

/// True when the provider has accepted a charge that settles later — the
/// asynchronous analogue of a card authorization.
///
/// Keyed on the rail: Stripe SEPA/ACH are as asynchronous as GoCardless, and a card is
/// synchronous through Stripe. A GoCardless mandate is a direct debit whatever its
/// scheme (becs, pad, autogiro… are stored as `Other`), so that provider is async on its
/// own. Stancer is async for CARDS too: an accepted charge lands `to_capture` (the
/// authorization succeeded, capture resolves later with no push channel), so a Pending
/// action-free Stancer charge is accepted-in-flight, never "awaiting the customer".
/// `next_action` present means the customer still has to do something (3DS), so
/// nothing has been accepted yet.
fn is_accepted_async_debit(
    transaction: &PaymentTransaction,
    method_type: &PaymentMethodTypeEnum,
    provider: &ConnectorProviderEnum,
) -> bool {
    transaction.status == PaymentStatusEnum::Pending
        && transaction.next_action.is_none()
        && (matches!(
            method_type,
            PaymentMethodTypeEnum::DirectDebitSepa
                | PaymentMethodTypeEnum::DirectDebitAch
                | PaymentMethodTypeEnum::DirectDebitBacs
        ) || matches!(
            provider,
            ConnectorProviderEnum::Gocardless | ConnectorProviderEnum::Stancer
        ))
}

impl Services {
    /// [`is_accepted_async_debit`] for a saved method: resolves the connector behind
    /// the method's connection so the rail check can account for the provider.
    pub(in crate::services) async fn accepted_async_debit(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        transaction: &PaymentTransaction,
        method: &CustomerPaymentMethodRow,
    ) -> StoreResult<bool> {
        // Cheap pre-check: only a Pending, action-free charge can qualify.
        if transaction.status != PaymentStatusEnum::Pending || transaction.next_action.is_some() {
            return Ok(false);
        }
        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, &tenant_id, &method.connection_id)
                .await
                .map_err(|e| StoreError::DatabaseError(e.error))?;
        Ok(is_accepted_async_debit(
            transaction,
            &method.payment_method_type,
            &connection.connector.provider,
        ))
    }

    pub(in crate::services) async fn bill_subscription_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        mode: InvoiceBillingMode,
    ) -> StoreResult<Option<DetailedInvoice>> {
        // TODO also check isFree for faster path

        let subscription = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;

        let customer =
            CustomerRow::find_by_id(conn, &subscription.subscription.customer_id, &tenant_id)
                .await
                .map_err(Into::into)
                .and_then(TryInto::try_into)?;

        self.bill_subscription_with_data_tx(conn, tenant_id, subscription, customer, mode)
            .await
    }

    pub(in crate::services) async fn bill_subscription_with_data_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription: SubscriptionDetails,
        customer: Customer,
        mode: InvoiceBillingMode,
    ) -> StoreResult<Option<DetailedInvoice>> {
        let draft_invoice = self
            .create_subscription_draft_invoice(
                conn,
                tenant_id,
                &subscription,
                customer.clone(), /* TODO */
            )
            .await?;

        let draft_invoice = if let Some(invoice) = draft_invoice {
            invoice
        } else {
            log::info!(
                "No draft invoice created for subscription {}. Skipping billing.",
                subscription.subscription.id
            );
            return Ok(None);
        };

        // A prior run may have already settled this period: create_subscription_draft_invoice
        // returns the existing recurring invoice for the date, which can be terminal (Finalized or
        // Closed). Idempotent for the scheduler modes — nothing left to bill, and finalizing again
        // would bail on `!can_edit()` — so return it as-is.
        //
        // NOT idempotent for the payment-carrying modes: they arrive with money already collected
        // (`AlreadyPaid`) or a confirmation to check before charging (`FinalizeAfterPayment`), and
        // skipping their arm would drop the transaction link and complete the checkout as if it had
        // been recorded. A terminal invoice there is an inconsistent state, so it must be loud.
        if !draft_invoice.can_edit() {
            return match &mode {
                InvoiceBillingMode::AwaitGracePeriodIfApplicable
                | InvoiceBillingMode::Immediate => {
                    self.as_detailed_invoice(draft_invoice, customer).map(Some)
                }
                InvoiceBillingMode::FinalizeAfterPayment { .. } => {
                    Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Cannot bill payment against invoice {} in terminal status {:?}",
                        draft_invoice.id, draft_invoice.status
                    )))
                }
                // Already collected: name the PSP charge, since it is now captured with no
                // transaction row anywhere and an operator has to reconcile it by hand.
                InvoiceBillingMode::AlreadyPaid { charge_result, .. } => {
                    Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Collected charge {} cannot be recorded against invoice {} in terminal status {:?}",
                        charge_result.payment_intent.external_id,
                        draft_invoice.id,
                        draft_invoice.status
                    )))
                }
                // Accepted debit in flight: the provider already created the
                // payment, so a terminal draft here would strand it unlinked.
                InvoiceBillingMode::AcceptedInFlight { existing_transaction_id, .. } => {
                    Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "In-flight charge (tx {}) cannot be recorded against invoice {} in terminal status {:?}",
                        existing_transaction_id,
                        draft_invoice.id,
                        draft_invoice.status
                    )))
                }
            };
        }

        let mut transactions = vec![];

        match mode {
            InvoiceBillingMode::FinalizeAfterPayment {
                currency_confirmation,
                total_amount_confirmation,
                payment_method_id,
            } => {
                if draft_invoice.currency != currency_confirmation {
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach("Currency is different from the confirmation"));
                }

                if draft_invoice.amount_due != (total_amount_confirmation as i64) {
                    return Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Total due amount is different from the confirmation : expected {}, got {}",
                        draft_invoice.amount_due, total_amount_confirmation
                    )));
                }

                // Also validates the method exists (it's already saved on the customer).
                // The rail decides whether settlement is synchronous, so it is needed on
                // both the zero-amount and the charged path.
                let payment_method =
                    CustomerPaymentMethodRow::get_by_id(conn, &tenant_id, &payment_method_id)
                        .await
                        .map_err(|e| StoreError::DatabaseError(e.error))?;

                // Handle zero amount case (e.g., 100% coupon discount)
                // No payment needed, just finalize
                if draft_invoice.amount_due == 0 {
                    // Finalize the invoice
                    self.finalize_invoice_tx(
                        conn,
                        &Actor::System,
                        draft_invoice.id,
                        tenant_id,
                        false,
                        &Some(subscription),
                    )
                    .await?;

                    // Mark as paid since amount is 0
                    InvoiceRow::apply_payment_status(
                        conn,
                        draft_invoice.id,
                        tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Paid,
                        None,
                    )
                    .await?;

                    // Get the updated invoice and return with no transactions
                    let updated_invoice =
                        InvoiceRow::find_detailed_by_id(conn, tenant_id, draft_invoice.id).await?;
                    let detailed = DetailedInvoice::try_from(updated_invoice)?;

                    // A zero-amount invoice is paid the moment it is finalized. Emit the same
                    // event as every other paid path, else webhooks-out and `on_invoice_paid`
                    // never learn about 100%-coupon checkouts.
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            &Actor::System,
                            vec![crate::domain::outbox_event::OutboxEvent::invoice_paid(
                                (&detailed.invoice).into(),
                            )],
                        )
                        .await?;

                    return Ok(Some(detailed.with_transactions(vec![])));
                }

                // Carry next_action on the transaction so the on-session caller
                // can surface 3DS; the invoice is finalized via the webhook once
                // payment settles.
                let (mut res, next_action) = self
                    .process_invoice_payment_tx(
                        conn,
                        tenant_id,
                        draft_invoice.id,
                        payment_method_id,
                        // FinalizeAfterPayment is only constructed by checkout
                        // completion: customer-initiated and on-session.
                        true,
                        true,
                        None,
                    )
                    .await?;
                res.next_action = next_action;

                transactions.push(res.clone());

                if res.status == PaymentStatusEnum::Settled {
                    // Payment succeeded - payment method is already saved on the customer
                    // Finalize the invoice directly
                    self.finalize_invoice_tx(
                        conn,
                        &Actor::System,
                        draft_invoice.id,
                        tenant_id,
                        false, // no need to refresh lines, we just paid
                        &Some(subscription),
                    )
                    .await?;
                } else if self
                    .accepted_async_debit(conn, tenant_id, &res, &payment_method)
                    .await?
                {
                    // Delayed-notification rail: the mandate is signed and the debit is
                    // submitted, so the amounts are contractual even though the funds take
                    // days to arrive. Finalize now — document state must not encode payment
                    // state — and record that money is moving so neither the auto-charge
                    // orchestration nor dunning touches this invoice.
                    self.finalize_invoice_tx(
                        conn,
                        &Actor::System,
                        draft_invoice.id,
                        tenant_id,
                        false, // amounts are fixed under the in-flight charge
                        &Some(subscription),
                    )
                    .await?;

                    InvoiceRow::apply_payment_status(
                        conn,
                        draft_invoice.id,
                        tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Processing,
                        None,
                    )
                    .await?;
                } else {
                    // Card 3DS (Pending with a next_action), or a non-terminal state we
                    // can't call accepted. Leave it draft; the webhook finalizes on settle.
                    return self
                        .as_detailed_invoice(draft_invoice, customer)
                        .map(|d| d.with_transactions(transactions))
                        .map(Some);
                }
            }
            InvoiceBillingMode::AwaitGracePeriodIfApplicable => {
                if !subscription.subscription.auto_advance_invoices {
                    // leave as draft
                    return self.as_detailed_invoice(draft_invoice, customer).map(Some);
                }

                let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                    conn,
                    subscription.subscription.invoicing_entity_id,
                    tenant_id,
                )
                .await?;

                // Schedule finalization after the grace period. Consolidation also requires the
                // scheduled path (the inline finalize below bypasses merging), so take it whenever
                // the entity consolidates, even with a negative grace.
                if invoicing_entity.grace_period_hours >= 0
                    || invoicing_entity.consolidate_recurring_invoices
                {
                    self.schedule_invoice_finalization(
                        conn,
                        tenant_id,
                        &subscription,
                        draft_invoice.id,
                        draft_invoice.invoice_date,
                        invoicing_entity.grace_period_hours,
                        invoicing_entity.consolidate_recurring_invoices,
                    )
                    .await?;

                    return self.as_detailed_invoice(draft_invoice, customer).map(Some);
                }

                // else we finalize immediately and trigger payment
                self.finalize_invoice_tx(
                    conn,
                    &Actor::System,
                    draft_invoice.id,
                    tenant_id,
                    false,
                    &Some(subscription.clone()),
                )
                .await?;
            }
            InvoiceBillingMode::Immediate => {
                if !subscription.subscription.auto_advance_invoices {
                    // leave as draft
                    return self.as_detailed_invoice(draft_invoice, customer).map(Some);
                }

                // Finalize and process payment immediately
                self.finalize_invoice_tx(
                    conn,
                    &Actor::System,
                    draft_invoice.id,
                    tenant_id,
                    false,
                    &Some(subscription.clone()),
                )
                .await?;

                // Handle zero amount case (e.g., 100% coupon discount)
                if draft_invoice.amount_due == 0 {
                    InvoiceRow::apply_payment_status(
                        conn,
                        draft_invoice.id,
                        tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Paid,
                        None,
                    )
                    .await?;
                }
            }
            InvoiceBillingMode::AlreadyPaid {
                charge_result,
                existing_transaction_id,
            } => {
                if draft_invoice.currency != charge_result.currency {
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach("Currency mismatch between invoice and payment"));
                }

                // Allow 1 subunit tolerance for rounding
                let amount_diff = (draft_invoice.amount_due - charge_result.amount).abs();
                if amount_diff > 1 {
                    return Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Amount mismatch: invoice {} vs payment {}",
                        draft_invoice.amount_due, charge_result.amount
                    )));
                }

                let transaction = if let Some(tx_id) = existing_transaction_id {
                    self.link_transaction_to_invoice(conn, tenant_id, tx_id, draft_invoice.id)
                        .await?
                } else {
                    self.create_transaction_for_direct_charge(
                        conn,
                        tenant_id,
                        draft_invoice.id,
                        &charge_result,
                        None,
                    )
                    .await?
                };

                transactions.push(transaction.clone());

                // Payment succeeded - payment method is already saved on the customer
                self.finalize_invoice_tx(
                    conn,
                    &Actor::System,
                    draft_invoice.id,
                    tenant_id,
                    false,
                    &Some(subscription.clone()),
                )
                .await?;

                // Apply the transaction amount to reduce amount_due and mark as paid
                let updated_invoice_row = InvoiceRow::apply_transaction(
                    conn,
                    draft_invoice.id,
                    tenant_id,
                    charge_result.amount,
                )
                .await?;

                if updated_invoice_row.amount_due == 0 {
                    InvoiceRow::apply_payment_status(
                        conn,
                        draft_invoice.id,
                        tenant_id,
                        diesel_models::enums::InvoicePaymentStatus::Paid,
                        transaction.processed_at,
                    )
                    .await?;

                    let invoice: Invoice = updated_invoice_row.try_into()?;
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            &Actor::System,
                            vec![crate::domain::outbox_event::OutboxEvent::invoice_paid(
                                (&invoice).into(),
                            )],
                        )
                        .await?;
                }
            }
            InvoiceBillingMode::AcceptedInFlight {
                existing_transaction_id,
                amount,
                currency,
            } => {
                if draft_invoice.currency != currency {
                    return Err(Report::new(StoreError::CheckoutError)
                        .attach("Currency mismatch between invoice and in-flight payment"));
                }

                // Allow 1 subunit tolerance for rounding.
                if (draft_invoice.amount_due - amount).abs() > 1 {
                    return Err(Report::new(StoreError::CheckoutError).attach(format!(
                        "Amount mismatch: invoice {} vs in-flight payment {}",
                        draft_invoice.amount_due, amount
                    )));
                }

                // Link the pre-created Pending transaction to the invoice so the
                // settlement webhook (`payments.confirmed`) routes through
                // `on_invoice_payment_settled` and flips Processing → Paid.
                let transaction = self
                    .link_transaction_to_invoice(
                        conn,
                        tenant_id,
                        existing_transaction_id,
                        draft_invoice.id,
                    )
                    .await?;
                transactions.push(transaction);

                // Amounts are contractual under the accepted debit — finalize now;
                // document state must not encode payment state.
                self.finalize_invoice_tx(
                    conn,
                    &Actor::System,
                    draft_invoice.id,
                    tenant_id,
                    false,
                    &Some(subscription.clone()),
                )
                .await?;

                // Processing (not Paid) and NO apply_transaction: the funds are in
                // flight, so amount_due stays full until real settlement. This also
                // fences the invoice off from auto-charge and dunning.
                InvoiceRow::apply_payment_status(
                    conn,
                    draft_invoice.id,
                    tenant_id,
                    diesel_models::enums::InvoicePaymentStatus::Processing,
                    None,
                )
                .await?;
            }
        }

        // Get the updated invoice after payment processing
        let updated_invoice =
            InvoiceRow::find_detailed_by_id(conn, tenant_id, draft_invoice.id).await?;

        Ok(Some(
            DetailedInvoice::try_from(updated_invoice)?.with_transactions(transactions),
        ))
    }

    fn as_detailed_invoice(
        &self,
        invoice: Invoice,
        customer: Customer,
    ) -> StoreResult<DetailedInvoice> {
        Ok(DetailedInvoice {
            invoice,
            plan: None, // TODO
            customer,
            transactions: vec![],
        })
    }

    /// Schedule invoice finalization after a grace period.
    ///
    /// When `consolidate` is set, the effective grace is floored to `CONSOLIDATION_MIN_GRACE_HOURS`
    /// so an aggressive (near-zero or negative) grace can't push the shared `invoice_date + grace`
    /// deadline into the past before all of a customer's same-day sibling drafts have been created —
    /// which would let one finalize alone and skip the merge. The deadline stays anchored to
    /// `invoice_date` (like grace), and usage is bounded by the closed billing period, so this never
    /// changes what is billed.
    #[allow(clippy::too_many_arguments)]
    async fn schedule_invoice_finalization(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription: &SubscriptionDetails,
        invoice_id: InvoiceId,
        invoice_date: chrono::NaiveDate,
        grace_period_hours: i32,
        consolidate: bool,
    ) -> StoreResult<()> {
        let effective_hours = if consolidate {
            std::cmp::max(grace_period_hours, CONSOLIDATION_MIN_GRACE_HOURS)
        } else {
            grace_period_hours
        };
        let scheduled_time = invoice_date.and_time(NaiveTime::MIN)
            + chrono::Duration::hours(i64::from(effective_hours));

        self.store
            .schedule_events(
                conn,
                vec![ScheduledEventNew {
                    subscription_id: subscription.subscription.id,
                    tenant_id,
                    scheduled_time,
                    event_data: ScheduledEventData::FinalizeInvoice { invoice_id },
                    source: String::new(),
                    created_by_customer: false,
                }],
            )
            .await?;

        Ok(())
    }
}
