use crate::StoreResult;
use crate::domain::customer_emails::{EmailContext, EmailKind};
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{InvoicePdfGeneratedEvent, OutboxEvent};
use crate::domain::pgmq::{PaymentRequestEvent, PgmqMessageNew, PgmqQueue};
use crate::domain::{Invoice, InvoicePaymentStatus, InvoiceType, ResolvedPaymentMethod};
use crate::repositories::customer_emails::CustomerEmailsInterface;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::repositories::pgmq::PgmqInterface;
use crate::repositories::{InvoiceInterface, SubscriptionInterface};
use crate::services::Services;
use chrono::Utc;
use common_domain::ids::TenantId;
use diesel_models::enums::{InvoicePaymentStatus as DieselPaymentStatus, InvoiceStatusEnum};
use diesel_models::invoices::InvoiceRow;
use scoped_futures::ScopedFutureExt;

impl Services {
    pub async fn on_invoice_accounting_pdf_generated(
        &self,
        event: InvoicePdfGeneratedEvent,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let invoice = self
            .store
            .get_invoice_by_id(tenant_id, event.invoice_id)
            .await?;

        // A consolidated (merged) invoice has no subscription_id of its own. Resolve the
        // payment method and auto-charge settings from one of its member subscriptions (all
        // members share the same resolved payment method by construction).
        let payment_subscription_id = match invoice.subscription_id {
            Some(id) => Some(id),
            None => self
                .store
                .list_consolidated_children(tenant_id, invoice.id)
                .await?
                .into_iter()
                .find_map(|child| child.subscription_id),
        };

        // Nothing to collect (100% coupon, downgrade credit, fully covered by credit): paid now,
        // whatever the invoice bills, so it can be credited or refunded like any paid invoice.
        if invoice.amount_due <= 0 && invoice.payment_status != InvoicePaymentStatus::Paid {
            self.mark_zero_amount_invoice_as_paid(tenant_id, &invoice)
                .await?;
            return Ok(());
        }

        if self.invoice_delivery_held(tenant_id, invoice.id).await? {
            return Ok(());
        }

        // Paid before its PDF existed (checkout, a card-charged top-up): `on_invoice_paid` found no
        // PDF, so the receipt goes out now, with or without a subscription.
        if invoice.payment_status == InvoicePaymentStatus::Paid {
            let receipt = self
                .store
                .last_settled_payment_tx_by_invoice_id(tenant_id, event.invoice_id)
                .await?;

            if receipt.is_some() {
                self.enqueue_invoice_email(tenant_id, invoice.id, EmailKind::InvoicePaid)
                    .await?;
                return Ok(());
            }
            if payment_subscription_id.is_some() {
                tracing::warn!("No receipt found for invoice {}", event.invoice_id);
                return Ok(());
            }
        }

        if let Some(subscription_id) = payment_subscription_id {
            // Unpaid: either send the invoice with its payment link (SendInvoice, or no usable
            // payment method), or trigger the automatic charge (the receipt follows its settlement).
            let subscription = self
                .store
                .get_subscription(tenant_id, subscription_id)
                .await?;

            // TODO should we save that in the invoice ? after it's paid only ?
            let subscription_payment_method = self
                .store
                .resolve_payment_method_for_subscription(tenant_id, subscription_id)
                .await?;

            // A payment already owns this invoice — an async debit accepted at checkout, or a
            // settlement whose Paid status hasn't been applied yet. Enqueueing another charge
            // would only be turned away by the in-flight guard of `claim_invoice_charge_tx`.
            if self
                .store
                .invoice_has_live_payment(tenant_id, invoice.id)
                .await?
            {
                return Ok(());
            }

            match (
                subscription_payment_method,
                subscription.charge_automatically,
            ) {
                (ResolvedPaymentMethod::CustomerPaymentMethod(payment_method_id), true) => {
                    // we trigger auto payment
                    let evt: PgmqMessageNew =
                        PaymentRequestEvent::new(tenant_id, event.invoice_id, payment_method_id)
                            .try_into()?;
                    self.request_auto_charge(&invoice, evt).await?;
                }
                // In all other cases, send the invoice ready email with a payment link.
                // The customer can pay via the portal using the link in the email.
                _ => {
                    self.enqueue_invoice_email(tenant_id, invoice.id, EmailKind::InvoiceReady)
                        .await?;
                }
            }
        } else if invoice.manual || self.awaits_payment_by_invoice(&invoice).await? {
            self.enqueue_invoice_email(tenant_id, invoice.id, EmailKind::InvoiceReady)
                .await?;
        }
        Ok(())
    }

    /// A one-off invoice without subscription (credit sold "send an invoice") is paid from its
    /// email. A top-up arms its charge before it is finalized, so it never reads as awaiting one.
    async fn awaits_payment_by_invoice(&self, invoice: &Invoice) -> StoreResult<bool> {
        if invoice.invoice_type != InvoiceType::OneOff
            || invoice.status != crate::domain::InvoiceStatusEnum::Finalized
            || invoice.payment_status == InvoicePaymentStatus::Paid
        {
            return Ok(false);
        }
        Ok(!self
            .store
            .invoice_has_any_payment(invoice.tenant_id, invoice.id)
            .await?)
    }

    /// An invoice a replay of past billing issued is neither collected nor sent.
    pub(crate) async fn invoice_delivery_held(
        &self,
        tenant_id: TenantId,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<bool> {
        let mut conn = self.store.get_conn().await?;
        diesel_models::replay_holds::is_invoice_delivery_held(&mut conn, tenant_id, invoice_id)
            .await
            .map_err(Into::into)
    }

    /// Slim job: the worker reloads invoice, customer, entity settings and PDF at send time.
    /// The key makes a redelivered PDF-generated event a no-op.
    pub(crate) async fn enqueue_invoice_email(
        &self,
        tenant_id: TenantId,
        invoice_id: common_domain::ids::InvoiceId,
        kind: EmailKind,
    ) -> StoreResult<()> {
        self.store
            .enqueue_customer_email(
                tenant_id,
                kind,
                EmailContext::Invoice { invoice_id },
                format!("{kind}:{invoice_id}"),
            )
            .await
    }

    /// The collection decision is stamped (once) in the same transaction that queues the
    /// charge, so an invoice never reads as auto-charged with no charge on the way.
    async fn request_auto_charge(&self, invoice: &Invoice, evt: PgmqMessageNew) -> StoreResult<()> {
        let (invoice_id, tenant_id) = (invoice.id, invoice.tenant_id);
        let stamp = invoice.auto_charge_requested_at.is_none();
        self.store
            .transaction(|conn| {
                async move {
                    if stamp {
                        InvoiceRow::set_auto_charge_requested(
                            conn,
                            invoice_id,
                            tenant_id,
                            Some(Utc::now().naive_utc()),
                        )
                        .await
                        .map_err(Into::<error_stack::Report<crate::errors::StoreError>>::into)?;
                    }
                    self.store
                        .pgmq_send_batch_tx(conn, PgmqQueue::PaymentRequest, vec![evt])
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    /// A queued auto-charge that can no longer run (its connector was disabled or deleted):
    /// send the invoice with its payment link instead, as if it had never been auto-charged.
    pub async fn fall_back_to_manual_payment(
        &self,
        tenant_id: TenantId,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> StoreResult<()> {
        let invoice = self.store.get_invoice_by_id(tenant_id, invoice_id).await?;
        {
            let mut conn = self.store.get_conn().await?;
            InvoiceRow::set_auto_charge_requested(&mut conn, invoice_id, tenant_id, None)
                .await
                .map_err(Into::<error_stack::Report<crate::errors::StoreError>>::into)?;
        }
        if invoice.pdf_document_id.is_none() {
            log::warn!("Invoice {invoice_id} has no PDF yet; no manual-payment email sent");
            return Ok(());
        }
        self.enqueue_invoice_email(tenant_id, invoice_id, EmailKind::InvoiceReady)
            .await
    }

    /// Handle $0 invoices (e.g., 100% coupon) - mark as paid and emit invoice_paid event.
    /// The `on_invoice_paid` handler will take care of subscription activation.
    async fn mark_zero_amount_invoice_as_paid(
        &self,
        tenant_id: TenantId,
        invoice: &Invoice,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                async move {
                    // Under the lock, so a redelivered event never pays it, nor emits, twice.
                    let current = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice.id)
                        .await?
                        .invoice;
                    if current.status != InvoiceStatusEnum::Finalized
                        || current.payment_status == DieselPaymentStatus::Paid
                        || current.amount_due > 0
                    {
                        return Ok(());
                    }
                    // Nothing to pay: paid the moment it was issued, not when this event ran.
                    let paid_at = current
                        .finalized_at
                        .unwrap_or_else(|| Utc::now().naive_utc());
                    let invoice = self
                        .apply_invoice_paid_tx(conn, tenant_id, invoice.id, Some(paid_at))
                        .await?;

                    // Emit invoice paid event - this triggers on_invoice_paid which handles
                    // subscription activation (TrialExpired → Active)
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            &Actor::System,
                            vec![OutboxEvent::invoice_paid((&invoice).into())],
                        )
                        .await?;

                    tracing::info!(
                        "Marked zero-amount invoice {} as paid (e.g., 100% coupon)",
                        invoice.id
                    );

                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }
}
