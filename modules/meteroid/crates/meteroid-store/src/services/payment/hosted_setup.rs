//! Hosted-setup completion — the server-side completion path for providers
//! whose hosted-redirect setup has NO webhook backstop
//! ([`crate::adapters::payment::HostedSetupCompletion::PollingRequired`]).
//! The return redirect (and the sweeper re-running this same routine) IS the
//! money path: complete the intent on the adapter, ownership-check its
//! metadata (unauthenticated endpoint — fail closed), upsert the card as
//! default, then handle the first payment. For invoice and checkout intents
//! the hosted page captured the REAL amount in-flow: that captured payment is
//! the SINGLE charge — its id is recorded on the pre-created transaction and
//! settled/materialized from, never a second server-initiated charge. Legacy
//! 0-amount intents keep the old fail-closed off-session charge. Lost returns
//! are backstopped by the pending-intent sweeper ([`super::hosted_payment_sweep`]).

use crate::StoreResult;
use crate::adapters::payment::error::ConnectorError;
use crate::adapters::payment::error::HostedSetupPending;
use crate::adapters::payment::{
    HostedSetupCompletion, PaymentConnector, initialize_payment_connector,
};
use crate::domain::connectors::Connector;
use crate::domain::entity_activity::Actor;
use crate::domain::{
    CustomerPatch, CustomerPaymentMethod, CustomerPaymentMethodNew, PaymentStatusEnum,
};
use crate::errors::StoreError;
use crate::repositories::CustomersInterface;
use crate::repositories::InvoiceInterface;
use crate::repositories::checkout_sessions::CheckoutSessionsInterface;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::Services;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CustomerConnectionId, CustomerPaymentMethodId, InvoiceId,
};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;
use std::time::Duration;

const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

/// The customer's redirect can beat the intent's own `.card` update — retry
/// briefly before surfacing "processing" (no webhook catches up later).
const COMPLETE_ATTEMPTS: u32 = 3;
const COMPLETE_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Customer-facing outcome of a hosted-setup return. System failures stay `Err`.
#[derive(Debug)]
pub enum HostedSetupOutcome {
    /// Card saved and set as default; the intent named no invoice/checkout.
    MethodSaved(CustomerPaymentMethod),
    /// Card saved and the named invoice's payment is in hand.
    InvoiceCharged(CustomerPaymentMethod),
    /// Card saved and the checkout's first payment accepted; subscription materialized.
    CheckoutActivated(CustomerPaymentMethod),
    /// Card saved but the first charge was declined; retryable with the saved card.
    PaymentFailed {
        payment_method: CustomerPaymentMethod,
        code: Option<String>,
    },
    /// Intent has no saved method after retries; a refresh re-runs this (idempotent).
    Processing,
    /// Hosted flow ended without a saved card (cancelled/unpaid/nonexistent intent).
    SetupFailed,
    /// Money WAS captured but does not reconcile with the transaction. Nothing
    /// settled; operator review required. Never expired by the sweeper —
    /// captured money is never cancelled away.
    HeldForReview {
        payment_method: CustomerPaymentMethod,
    },
}

impl Services {
    /// Release the pending-intent marker of a FINISHED attempt so it stops
    /// being swept and is never adoptable. Predicated on the observed intent id.
    async fn release_hosted_intent_marker(
        &self,
        tenant_id: common_domain::ids::TenantId,
        row: &diesel_models::payments::PaymentTransactionRow,
    ) -> StoreResult<()> {
        use diesel_models::payments::PaymentTransactionRow;

        let Some(marker) = row.pending_provider_intent_id.as_deref() else {
            return Ok(());
        };
        let mut conn = self.store.get_conn().await?;
        PaymentTransactionRow::clear_pending_intent_if_matches(
            &mut conn, tenant_id, row.id, marker,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;
        Ok(())
    }

    /// Finalize a hosted setup intent after the customer returns, then perform
    /// the fail-closed first payment. Idempotent across re-visits.
    /// Unauthenticated, attacker-supplied ids: the intent metadata is
    /// ownership-checked before anything is attached or charged.
    pub async fn complete_hosted_setup(
        &self,
        connection_id: CustomerConnectionId,
        intent_id: String,
    ) -> StoreResult<HostedSetupOutcome> {
        self.complete_hosted_setup_with_attempts(connection_id, intent_id, COMPLETE_ATTEMPTS)
            .await
    }

    /// [`Self::complete_hosted_setup`] with an explicit `.card`-timing retry
    /// budget: the return handler retries briefly; the sweeper passes 1.
    pub(crate) async fn complete_hosted_setup_with_attempts(
        &self,
        connection_id: CustomerConnectionId,
        intent_id: String,
        complete_attempts: u32,
    ) -> StoreResult<HostedSetupOutcome> {
        let mut conn = self.store.get_conn().await?;

        let connection_row =
            CustomerConnectionDetailsRow::get_by_id_unscoped(&mut conn, &connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;
        drop(conn);

        let external_customer_id = connection_row.external_customer_id.clone();
        let connector =
            Connector::from_row(&self.store.settings.crypt_key, connection_row.connector)?;

        // Webhook-backed providers complete through their webhook and never
        // persist a sweepable intent id — refuse to run this money path for them.
        let polling_required = crate::adapters::payment::provider_capabilities(&connector.provider)
            .is_some_and(|caps| {
                caps.hosted_setup_completion == HostedSetupCompletion::PollingRequired
            });
        if !polling_required {
            return Err(Report::new(StoreError::InvalidArgument(
                "connection's provider does not use polled hosted-setup completion".to_string(),
            )));
        }

        let tenant_id = connector.tenant_id;
        let customer_id = connection_row.customer.id;

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let mut snapshot = None;
        let complete_attempts = complete_attempts.max(1);
        for attempt in 1..=complete_attempts {
            let result = tokio::time::timeout(
                PAYMENT_PROVIDER_TIMEOUT,
                connector_impl.complete_mandate_setup(&connector, &intent_id),
            )
            .await
            .map_err(|_| {
                Report::new(StoreError::PaymentProviderError)
                    .attach("Payment provider request timed out")
            })?;

            match result {
                Ok(s) => {
                    snapshot = Some(s);
                    break;
                }
                Err(report) => {
                    let pending = report
                        .frames()
                        .any(|f| f.downcast_ref::<HostedSetupPending>().is_some());
                    if pending {
                        if attempt < complete_attempts {
                            tokio::time::sleep(COMPLETE_RETRY_DELAY).await;
                            continue;
                        }
                        log::info!(
                            "hosted setup intent {intent_id} still has no payment method after {complete_attempts} attempts; reporting processing"
                        );
                        return Ok(HostedSetupOutcome::Processing);
                    }
                    // Terminal setup failure → customer-facing "failed";
                    // anything else (transport, config) is a system error.
                    return if matches!(report.current_context(), ConnectorError::MandateSetup(_)) {
                        log::info!(
                            "hosted setup for intent {intent_id} did not complete: {report:?}"
                        );
                        Ok(HostedSetupOutcome::SetupFailed)
                    } else {
                        Err(report.change_context(StoreError::PaymentProviderError))
                    };
                }
            }
        }
        let snapshot = snapshot.expect("loop either sets a snapshot or returns");

        // ── hijack defense: the intent must name THIS connection+customer ──
        let expected_connection = connection_id.as_base62();
        let expected_customer = customer_id.as_base62();
        match (
            snapshot.meteroid_connection_id.as_deref(),
            snapshot.meteroid_customer_id.as_deref(),
        ) {
            (Some(conn_id), Some(cust))
                if conn_id == expected_connection && cust == expected_customer => {}
            other => {
                return Err(Report::new(StoreError::InvalidArgument(
                    "hosted setup intent does not belong to this connection".to_string(),
                ))
                .attach(format!(
                    "expected connection={expected_connection} customer={expected_customer}, \
                     intent carried {other:?}"
                )));
            }
        }

        let invoice_to_charge = snapshot.meteroid_invoice_id.clone();
        let checkout_session_to_complete = snapshot.meteroid_checkout_session_id.clone();
        // In-flow capture: present ⇒ record it, NEVER charge again.
        let captured_payment_id = snapshot.payment_request_payment.clone();
        // The intent's OWN pre-created transaction — the capture is recorded
        // onto this row, never "the latest" (which can be a newer attempt).
        let intent_transaction_id = snapshot.meteroid_transaction_id.clone();
        let external_payment_method_id = snapshot.external_payment_method_id.clone();

        let payment_method = self
            .store
            .upsert_payment_method(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id,
                customer_id,
                connection_id,
                external_payment_method_id,
                payment_method_type: snapshot.payment_method_type,
                account_number_hint: snapshot.account_number_hint,
                card_brand: snapshot.card_brand,
                card_last4: snapshot.card_last4,
                card_exp_month: snapshot.card_exp_month,
                card_exp_year: snapshot.card_exp_year,
            })
            .await?;

        let patch = CustomerPatch {
            id: customer_id,
            name: None,
            alias: None,
            billing_email: None,
            phone: None,
            balance_value_cents: None,
            currency: None,
            billing_address: None,
            shipping_address: None,
            invoicing_entity_id: None,
            vat_number: None,
            current_payment_method_id: Some(Some(payment_method.id)),
            invoicing_emails: None,
            is_tax_exempt: None,
            custom_taxes: None,
            connected_account_id: None,
        };
        self.store
            .patch_customer(Actor::System, tenant_id, patch)
            .await?;

        if let Some(invoice_id_str) = invoice_to_charge {
            self.settle_invoice_after_hosted_setup(
                tenant_id,
                customer_id,
                &connector,
                connector_impl.as_ref(),
                payment_method,
                &invoice_id_str,
                captured_payment_id,
                intent_transaction_id,
            )
            .await
        } else if let Some(session_id_str) = checkout_session_to_complete {
            self.activate_checkout_after_hosted_setup(
                tenant_id,
                customer_id,
                &external_customer_id,
                &connector,
                connector_impl.as_ref(),
                payment_method,
                &session_id_str,
                captured_payment_id,
                intent_transaction_id,
            )
            .await
        } else {
            Ok(HostedSetupOutcome::MethodSaved(payment_method))
        }
    }

    /// Settle (or, legacy-only, charge) the invoice the setup was made for.
    /// An in-flow intent's captured payment is recorded and settled — never
    /// followed by an off-session charge (double-collect). Only legacy
    /// 0-amount intents keep the fail-closed off-session charge.
    #[allow(clippy::too_many_arguments)]
    async fn settle_invoice_after_hosted_setup(
        &self,
        tenant_id: common_domain::ids::TenantId,
        customer_id: common_domain::ids::CustomerId,
        connector: &Connector,
        connector_impl: &dyn PaymentConnector,
        payment_method: CustomerPaymentMethod,
        invoice_id_str: &str,
        captured_payment_id: Option<String>,
        intent_transaction_id: Option<String>,
    ) -> StoreResult<HostedSetupOutcome> {
        use diesel_models::payments::PaymentTransactionRow;

        let invoice_id = match InvoiceId::parse_base62(invoice_id_str) {
            Ok(id) => id,
            Err(_) => {
                log::error!(
                    "hosted setup intent metadata carried an unparseable invoice id: {invoice_id_str}"
                );
                return Ok(HostedSetupOutcome::MethodSaved(payment_method));
            }
        };

        // The invoice id was read back from provider metadata (no auth
        // context): re-verify it belongs to this customer before moving money.
        let invoice = self.store.get_invoice_by_id(tenant_id, invoice_id).await?;
        if invoice.customer_id != customer_id {
            log::error!(
                "refusing to settle invoice {} for payment method {}: invoice customer {} != method customer {}",
                invoice_id,
                payment_method.id,
                invoice.customer_id.as_base62(),
                customer_id.as_base62(),
            );
            return Ok(HostedSetupOutcome::MethodSaved(payment_method));
        }

        match invoice_setup_settlement(
            intent_transaction_id.is_some(),
            captured_payment_id.is_some(),
        ) {
            InvoiceSetupSettlement::LegacyOffSessionCharge => {
                self.charge_invoice_after_hosted_setup(tenant_id, payment_method, invoice_id)
                    .await
            }
            InvoiceSetupSettlement::AwaitCapture => {
                // NEVER charge here — the hosted capture may still land;
                // the sweeper / a refresh re-runs completion.
                log::info!(
                    "hosted invoice setup for invoice {invoice_id}: in-flow intent has no \
                     captured payment yet; reporting processing"
                );
                Ok(HostedSetupOutcome::Processing)
            }
            InvoiceSetupSettlement::HoldUnmappedCapture => {
                log::error!(
                    "hosted invoice setup for invoice {invoice_id}: captured payment {:?} on an \
                     intent without meteroid.transaction_id; manual review required",
                    captured_payment_id
                );
                Ok(HostedSetupOutcome::HeldForReview { payment_method })
            }
            InvoiceSetupSettlement::RecordCapture => {
                let tx_id_str =
                    intent_transaction_id.expect("RecordCapture requires a transaction id");
                let payment_id =
                    captured_payment_id.expect("RecordCapture requires a captured payment");

                let Ok(tx_id) = common_domain::ids::PaymentTransactionId::parse_base62(&tx_id_str)
                else {
                    log::error!(
                        "hosted invoice setup for invoice {invoice_id}: intent carried an \
                         unparseable meteroid.transaction_id ({tx_id_str}); manual review required"
                    );
                    return Ok(HostedSetupOutcome::HeldForReview { payment_method });
                };
                let mut conn = self.store.get_conn().await?;
                let row = PaymentTransactionRow::get_by_id(&mut conn, tx_id, tenant_id)
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;
                drop(conn);
                if row.invoice_id != Some(invoice_id) {
                    // The tx exists but belongs elsewhere: never move money across it.
                    log::error!(
                        "hosted invoice setup for invoice {invoice_id}: intent transaction {} \
                         belongs to invoice {:?}; manual review required",
                        row.id,
                        row.invoice_id
                    );
                    return Ok(HostedSetupOutcome::HeldForReview { payment_method });
                }

                // Duplicate visit: already settled.
                if row.status == diesel_models::enums::PaymentStatusEnum::Settled
                    || row.status == diesel_models::enums::PaymentStatusEnum::Refunded
                {
                    if row.provider_transaction_id.as_deref() != Some(payment_id.as_str()) {
                        // A capture that is NOT the one recorded on the row is
                        // a SECOND capture at the provider — never drop it.
                        log::error!(
                            "hosted invoice setup for invoice {invoice_id}: captured payment \
                             {payment_id} arrived for already-settled transaction {} (provider \
                             id {:?}) — a second capture exists at the provider; manual \
                             review/refund required",
                            row.id,
                            row.provider_transaction_id
                        );
                    }
                    // Release any marker a reconcile-settled row may still carry.
                    self.release_hosted_intent_marker(tenant_id, &row).await?;
                    return Ok(HostedSetupOutcome::InvoiceCharged(payment_method));
                }
                if matches!(
                    row.status,
                    diesel_models::enums::PaymentStatusEnum::Failed
                        | diesel_models::enums::PaymentStatusEnum::Cancelled
                ) {
                    if row.provider_transaction_id.as_deref() != Some(payment_id.as_str()) {
                        // Late return after the sweeper closed the attempt, with
                        // money captured under a different id: manual review/refund.
                        log::error!(
                            "hosted invoice setup for invoice {invoice_id}: captured payment \
                             {payment_id} arrived for terminal transaction {} ({:?}, provider id \
                             {:?}); manual review required (refund or manual settlement)",
                            row.id,
                            row.status,
                            row.provider_transaction_id
                        );
                        return Ok(HostedSetupOutcome::HeldForReview { payment_method });
                    }
                    return Ok(HostedSetupOutcome::PaymentFailed {
                        payment_method,
                        code: row.error_type.clone(),
                    });
                }

                self.record_captured_invoice_payment(
                    tenant_id,
                    invoice_id,
                    connector,
                    connector_impl,
                    payment_method,
                    &row,
                    payment_id,
                )
                .await
            }
        }
    }

    /// Settle the invoice FROM the in-flow captured payment — never charge;
    /// settling drives the invoice Paid pipeline via the settlement outbox event.
    #[allow(clippy::too_many_arguments)]
    async fn record_captured_invoice_payment(
        &self,
        tenant_id: common_domain::ids::TenantId,
        invoice_id: InvoiceId,
        connector: &Connector,
        connector_impl: &dyn PaymentConnector,
        payment_method: CustomerPaymentMethod,
        row: &diesel_models::payments::PaymentTransactionRow,
        payment_id: String,
    ) -> StoreResult<HostedSetupOutcome> {
        use crate::adapters::payment::model::RemoteTransactionStatus;
        use crate::domain::payment_transactions::PaymentIntent;

        let remote = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.fetch_transaction_status(connector, &payment_id),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?
        .change_context(StoreError::PaymentProviderError)?;

        if matches!(&remote, RemoteTransactionStatus::Unknown) {
            log::error!(
                "hosted invoice payment for invoice {invoice_id}: intent-captured payment \
                 {payment_id} not found at provider; recording for manual review"
            );
        }

        match resolve_captured_payment(remote, row.amount, &row.currency) {
            CapturedPaymentResolution::Declined { code, message } => {
                let intent = PaymentIntent {
                    external_id: payment_id,
                    transaction_id: row.id,
                    tenant_id,
                    amount_requested: row.amount,
                    amount_received: None,
                    currency: row.currency.clone(),
                    next_action: None,
                    status: PaymentStatusEnum::Failed,
                    last_payment_error: Some(message.clone()),
                    processed_at: None,
                };
                self.consolidate_hosted_intent(tenant_id, row.id, intent, Some(payment_method.id))
                    .await?;
                log::warn!(
                    "hosted invoice payment for invoice {invoice_id}: in-flow captured payment \
                     was declined ({message})"
                );
                Ok(HostedSetupOutcome::PaymentFailed {
                    payment_method,
                    code,
                })
            }
            CapturedPaymentResolution::Cancelled => {
                let intent = PaymentIntent {
                    external_id: payment_id,
                    transaction_id: row.id,
                    tenant_id,
                    amount_requested: row.amount,
                    amount_received: None,
                    currency: row.currency.clone(),
                    next_action: None,
                    status: PaymentStatusEnum::Cancelled,
                    last_payment_error: None,
                    processed_at: None,
                };
                self.consolidate_hosted_intent(tenant_id, row.id, intent, Some(payment_method.id))
                    .await?;
                Ok(HostedSetupOutcome::PaymentFailed {
                    payment_method,
                    code: None,
                })
            }
            CapturedPaymentResolution::SettleNow {
                amount_received_minor,
                processed_at,
            } => {
                let intent = PaymentIntent {
                    external_id: payment_id.clone(),
                    transaction_id: row.id,
                    tenant_id,
                    amount_requested: row.amount,
                    amount_received: Some(amount_received_minor),
                    currency: row.currency.clone(),
                    next_action: None,
                    status: PaymentStatusEnum::Settled,
                    last_payment_error: None,
                    processed_at: Some(processed_at),
                };
                let final_tx = self
                    .consolidate_hosted_intent(tenant_id, row.id, intent, Some(payment_method.id))
                    .await?;
                // Consolidation skips terminal rows: if the sweeper cancelled
                // this tx meanwhile, nothing settled — hold (money IS captured).
                if final_tx.status != crate::domain::PaymentStatusEnum::Settled {
                    log::error!(
                        "hosted invoice payment for invoice {invoice_id}: captured payment \
                         {payment_id} could not settle transaction {} (now {:?}); manual review \
                         required",
                        row.id,
                        final_tx.status
                    );
                    return Ok(HostedSetupOutcome::HeldForReview { payment_method });
                }
                // The settlement outbox event marks the invoice Paid.
                Ok(HostedSetupOutcome::InvoiceCharged(payment_method))
            }
            CapturedPaymentResolution::HoldMismatch {
                amount_received_minor,
                remote_currency,
            } => {
                log::error!(
                    "hosted invoice payment for invoice {invoice_id}: captured payment \
                     {payment_id} reports {amount_received_minor} {remote_currency} but \
                     transaction {} expects {} {}; holding for manual review — NOT settling",
                    row.id,
                    row.amount,
                    row.currency
                );
                Ok(HostedSetupOutcome::HeldForReview { payment_method })
            }
            CapturedPaymentResolution::RecordPending => {
                // Bind id + method; reconcile polls the still-Pending tx to settlement.
                let intent = PaymentIntent {
                    external_id: payment_id,
                    transaction_id: row.id,
                    tenant_id,
                    amount_requested: row.amount,
                    amount_received: None,
                    currency: row.currency.clone(),
                    next_action: None,
                    status: PaymentStatusEnum::Pending,
                    last_payment_error: None,
                    processed_at: None,
                };
                self.consolidate_hosted_intent(tenant_id, row.id, intent, Some(payment_method.id))
                    .await?;
                Ok(HostedSetupOutcome::InvoiceCharged(payment_method))
            }
        }
    }

    /// LEGACY 0-amount intents only: charge synchronously on the return path.
    /// In-flow intents record the hosted capture and must never reach this.
    async fn charge_invoice_after_hosted_setup(
        &self,
        tenant_id: common_domain::ids::TenantId,
        payment_method: CustomerPaymentMethod,
        // Already parsed and ownership-checked by the caller.
        invoice_id: InvoiceId,
    ) -> StoreResult<HostedSetupOutcome> {
        let payment_method_id = payment_method.id;
        let charge_result = self
            .store
            .transaction(|conn| {
                async move {
                    self.process_invoice_payment_tx(
                        conn,
                        tenant_id,
                        invoice_id,
                        payment_method_id,
                        // Customer-initiated: they just completed the hosted flow.
                        true,
                        // Off-session: the hosted page already ran any 3DS.
                        false,
                        // No explicit ref: invoice charges derive a stable
                        // (method, invoice, attempt) key in `create_payment_intent`.
                        None,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await;

        match charge_result {
            Ok((tx, _)) => match tx.status {
                PaymentStatusEnum::Failed | PaymentStatusEnum::Cancelled => {
                    log::warn!(
                        "payment method {} saved but invoice {} first charge was declined: {:?}",
                        payment_method.id,
                        invoice_id,
                        tx.error_type
                    );
                    Ok(HostedSetupOutcome::PaymentFailed {
                        payment_method,
                        code: tx.error_type,
                    })
                }
                _ => {
                    log::info!(
                        "payment method {} saved; invoice {} charge initiated (tx {}, {:?})",
                        payment_method.id,
                        invoice_id,
                        tx.id,
                        tx.status
                    );
                    Ok(HostedSetupOutcome::InvoiceCharged(payment_method))
                }
            },
            // Benign duplicate visit: already pending/sufficient/non-payable.
            Err(e)
                if matches!(
                    e.current_context(),
                    StoreError::PaymentError(_) | StoreError::BillingError
                ) =>
            {
                log::info!(
                    "payment method {} saved; invoice {} not charged (already paid/pending/non-payable): {e:?}",
                    payment_method.id,
                    invoice_id
                );
                Ok(HostedSetupOutcome::InvoiceCharged(payment_method))
            }
            Err(e) => Err(e),
        }
    }

    /// Complete the checkout against the pre-created Pending transaction and
    /// materialize the subscription. The in-flow captured payment is the
    /// SINGLE charge: record its id and drive `on_hosted_checkout_fulfilled`
    /// (re-visits / the racing sweeper re-run idempotently). Legacy 0-amount
    /// intents keep the fail-closed off-session charge, anchored on the stable
    /// transaction id so a re-visit can never double-charge.
    #[allow(clippy::too_many_arguments)]
    async fn activate_checkout_after_hosted_setup(
        &self,
        tenant_id: common_domain::ids::TenantId,
        customer_id: common_domain::ids::CustomerId,
        external_customer_id: &str,
        connector: &Connector,
        connector_impl: &dyn PaymentConnector,
        payment_method: CustomerPaymentMethod,
        session_id_str: &str,
        captured_payment_id: Option<String>,
        intent_transaction_id: Option<String>,
    ) -> StoreResult<HostedSetupOutcome> {
        use crate::adapters::payment::bridge::payment_intent_from_outcome;
        use crate::adapters::payment::model::{ChargeRequest, IdempotencyKey};
        use crate::domain::PaymentMethodTypeEnum;
        use diesel_models::payments::PaymentTransactionRow;

        let checkout_session_id = match CheckoutSessionId::parse_base62(session_id_str) {
            Ok(id) => id,
            Err(_) => {
                log::error!(
                    "hosted setup intent metadata carried an unparseable checkout session id: {session_id_str}"
                );
                return Ok(HostedSetupOutcome::MethodSaved(payment_method));
            }
        };

        // Verify the session belongs to this customer before moving money.
        let session = self
            .store
            .get_checkout_session(tenant_id, checkout_session_id)
            .await?;
        if session.customer_id != customer_id {
            log::error!(
                "refusing to complete checkout session {} for payment method {}: session customer {} != method customer {}",
                checkout_session_id,
                payment_method.id,
                session.customer_id.as_base62(),
                customer_id.as_base62(),
            );
            return Ok(HostedSetupOutcome::MethodSaved(payment_method));
        }

        let mut conn = self.store.get_conn().await?;
        // Resolve the intent's OWN transaction via its stamped id: "latest for
        // the session" could record onto the wrong (newer) row after a retry.
        // The latest-row fallback exists only for legacy unstamped intents.
        let row = match &intent_transaction_id {
            Some(tx_id_str) => {
                let Ok(tx_id) = common_domain::ids::PaymentTransactionId::parse_base62(tx_id_str)
                else {
                    log::error!(
                        "hosted checkout session {checkout_session_id}: intent carried an \
                         unparseable meteroid.transaction_id ({tx_id_str}); manual review required"
                    );
                    return Ok(HostedSetupOutcome::HeldForReview { payment_method });
                };
                let row = PaymentTransactionRow::get_by_id(&mut conn, tx_id, tenant_id)
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;
                if row.checkout_session_id != Some(checkout_session_id) {
                    // The tx exists but belongs elsewhere: never move money across it.
                    log::error!(
                        "hosted checkout session {checkout_session_id}: intent transaction {} \
                         belongs to session {:?}; manual review required",
                        row.id,
                        row.checkout_session_id
                    );
                    return Ok(HostedSetupOutcome::HeldForReview { payment_method });
                }
                Some(row)
            }
            None => {
                let row = PaymentTransactionRow::get_latest_by_checkout_session_id(
                    &mut conn,
                    checkout_session_id,
                    tenant_id,
                )
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;
                if row.is_some() {
                    log::info!(
                        "hosted checkout session {checkout_session_id}: intent carries no \
                         meteroid.transaction_id (legacy); falling back to the session's latest \
                         transaction"
                    );
                }
                row
            }
        };
        let Some(row) = row else {
            // initiate_hosted_checkout pre-creates this row; card stays saved.
            log::warn!(
                "hosted checkout return for session {checkout_session_id}: no checkout transaction found; card saved only"
            );
            return Ok(HostedSetupOutcome::MethodSaved(payment_method));
        };
        drop(conn);

        // Duplicate visit: already materialized, or already terminal.
        if row.invoice_id.is_some()
            || row.status == diesel_models::enums::PaymentStatusEnum::Settled
        {
            // A captured payment that is NOT the recorded one is a SECOND
            // capture at the provider — never drop it silently.
            if let Some(payment_id) = &captured_payment_id
                && row.provider_transaction_id.as_deref() != Some(payment_id.as_str())
            {
                log::error!(
                    "hosted checkout session {checkout_session_id}: captured payment \
                     {payment_id} arrived for already-progressed transaction {} ({:?}, provider \
                     id {:?}) — a second capture exists at the provider; manual review/refund \
                     required",
                    row.id,
                    row.status,
                    row.provider_transaction_id
                );
            }
            log::info!(
                "hosted checkout session {checkout_session_id}: transaction {} already progressed ({:?}); re-running idempotent materialization",
                row.id,
                row.status
            );
            self.on_hosted_checkout_fulfilled(
                tenant_id,
                checkout_session_id,
                payment_method.id,
                row.provider_transaction_id.clone(),
                row.processed_at,
            )
            .await?;
            // Settled + materialized = finished; in-flight rows keep the marker.
            if row.status == diesel_models::enums::PaymentStatusEnum::Settled {
                self.release_hosted_intent_marker(tenant_id, &row).await?;
            }
            return Ok(HostedSetupOutcome::CheckoutActivated(payment_method));
        }
        if matches!(
            row.status,
            diesel_models::enums::PaymentStatusEnum::Failed
                | diesel_models::enums::PaymentStatusEnum::Cancelled
                | diesel_models::enums::PaymentStatusEnum::Refunded
        ) {
            if let Some(payment_id) = &captured_payment_id
                && row.provider_transaction_id.as_deref() != Some(payment_id.as_str())
            {
                // Money captured but the tx is terminal under a different id —
                // late return after sweeper expiry. Never drop captured funds.
                log::error!(
                    "hosted checkout session {checkout_session_id}: captured payment {payment_id} \
                     arrived for terminal transaction {} ({:?}, provider id {:?}); manual review \
                     required (refund or manual materialization)",
                    row.id,
                    row.status,
                    row.provider_transaction_id
                );
            }
            return Ok(HostedSetupOutcome::PaymentFailed {
                payment_method,
                code: row.error_type.clone(),
            });
        }

        // ── in-flow capture: record the hosted page's payment, never charge ──
        if let Some(payment_id) = captured_payment_id {
            return self
                .record_captured_checkout_payment(
                    tenant_id,
                    checkout_session_id,
                    connector,
                    connector_impl,
                    payment_method,
                    &row,
                    payment_id,
                )
                .await;
        }

        // ── legacy 0-amount intent: the fail-closed first charge ────────
        // The idempotency key rides on the stable checkout transaction id, so a
        // racing re-visit dedupes at the provider (unique_id conflict → adopt).
        let outcome = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.charge_off_session(
                connector,
                ChargeRequest {
                    transaction_id: row.id,
                    customer_external_id: external_customer_id,
                    payment_method_external_id: &payment_method.external_payment_method_id,
                    payment_method_type: PaymentMethodTypeEnum::Card,
                    amount_minor: row.amount,
                    currency: &row.currency,
                    idempotency_key: IdempotencyKey::new(format!("charge:{}", row.id.as_base62())),
                    // Off-session: 3DS already ran on the hosted page.
                    on_session: false,
                },
            ),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?
        .change_context(StoreError::PaymentProviderError)?;

        let intent = payment_intent_from_outcome(
            outcome,
            row.id,
            tenant_id,
            row.amount,
            row.currency.clone(),
        );
        let charge_status = intent.status.clone();
        let charge_error = intent.last_payment_error.clone();
        let provider_payment_id = Some(intent.external_id.clone()).filter(|s| !s.is_empty());
        let processed_at = intent.processed_at;

        let transaction_id = row.id;
        let store = self.store.clone();
        self.store
            .transaction(|conn| {
                let store = store.clone();
                let intent = intent.clone();
                async move {
                    let existing = store
                        .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                        .await?;
                    store
                        .consolidate_intent_and_transaction_tx(
                            conn,
                            &Actor::System,
                            existing,
                            intent,
                        )
                        .await?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await?;

        match charge_status {
            PaymentStatusEnum::Failed | PaymentStatusEnum::Cancelled => {
                log::warn!(
                    "payment method {} saved but checkout session {} first charge was declined: {:?}",
                    payment_method.id,
                    checkout_session_id,
                    charge_error
                );
                Ok(HostedSetupOutcome::PaymentFailed {
                    payment_method,
                    code: charge_error,
                })
            }
            _ => {
                // Accepted (Pending/Settled): bind method + payment, materialize.
                self.on_hosted_checkout_fulfilled(
                    tenant_id,
                    checkout_session_id,
                    payment_method.id,
                    provider_payment_id,
                    processed_at,
                )
                .await?;
                // Settled ⇒ finished — release the marker; in-flight keeps it.
                if charge_status == PaymentStatusEnum::Settled {
                    self.release_hosted_intent_marker(tenant_id, &row).await?;
                }
                Ok(HostedSetupOutcome::CheckoutActivated(payment_method))
            }
        }
    }

    /// Settle the checkout FROM the in-flow captured payment — never charge.
    /// Settled → consolidate + materialize Paid; still capturing → record the
    /// id, materialize Processing, reconcile later; declined/cancelled →
    /// consolidate the failure for a retry. Idempotent with the sweeper.
    #[allow(clippy::too_many_arguments)]
    async fn record_captured_checkout_payment(
        &self,
        tenant_id: common_domain::ids::TenantId,
        checkout_session_id: CheckoutSessionId,
        connector: &Connector,
        connector_impl: &dyn PaymentConnector,
        payment_method: CustomerPaymentMethod,
        row: &diesel_models::payments::PaymentTransactionRow,
        payment_id: String,
    ) -> StoreResult<HostedSetupOutcome> {
        use crate::adapters::payment::model::RemoteTransactionStatus;
        use crate::domain::payment_transactions::PaymentIntent;

        let remote = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.fetch_transaction_status(connector, &payment_id),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?
        .change_context(StoreError::PaymentProviderError)?;

        if matches!(&remote, RemoteTransactionStatus::Unknown) {
            // The id came from the intent itself: a 404 is an env mismatch,
            // not a lost payment — record it and let reconcile hold it.
            log::error!(
                "hosted checkout session {checkout_session_id}: intent-captured payment \
                 {payment_id} not found at provider; recording for manual review"
            );
        }

        match resolve_captured_payment(remote, row.amount, &row.currency) {
            CapturedPaymentResolution::Declined { code, message } => {
                let intent = PaymentIntent {
                    external_id: payment_id,
                    transaction_id: row.id,
                    tenant_id,
                    amount_requested: row.amount,
                    amount_received: None,
                    currency: row.currency.clone(),
                    next_action: None,
                    status: PaymentStatusEnum::Failed,
                    last_payment_error: Some(message.clone()),
                    processed_at: None,
                };
                self.consolidate_hosted_checkout_intent(tenant_id, row.id, intent)
                    .await?;
                log::warn!(
                    "hosted checkout session {checkout_session_id}: in-flow captured payment \
                     was declined ({message})"
                );
                Ok(HostedSetupOutcome::PaymentFailed {
                    payment_method,
                    code,
                })
            }
            CapturedPaymentResolution::Cancelled => {
                let intent = PaymentIntent {
                    external_id: payment_id,
                    transaction_id: row.id,
                    tenant_id,
                    amount_requested: row.amount,
                    amount_received: None,
                    currency: row.currency.clone(),
                    next_action: None,
                    status: PaymentStatusEnum::Cancelled,
                    last_payment_error: None,
                    processed_at: None,
                };
                self.consolidate_hosted_checkout_intent(tenant_id, row.id, intent)
                    .await?;
                Ok(HostedSetupOutcome::PaymentFailed {
                    payment_method,
                    code: None,
                })
            }
            CapturedPaymentResolution::SettleNow {
                amount_received_minor,
                processed_at,
            } => {
                let intent = PaymentIntent {
                    external_id: payment_id.clone(),
                    transaction_id: row.id,
                    tenant_id,
                    amount_requested: row.amount,
                    amount_received: Some(amount_received_minor),
                    currency: row.currency.clone(),
                    next_action: None,
                    status: PaymentStatusEnum::Settled,
                    last_payment_error: None,
                    processed_at: Some(processed_at),
                };
                let final_tx = self
                    .consolidate_hosted_checkout_intent(tenant_id, row.id, intent)
                    .await?;
                // Consolidation skips terminal rows: if the sweeper cancelled
                // this tx meanwhile, nothing settled — hold (money IS captured).
                if final_tx.status != crate::domain::PaymentStatusEnum::Settled {
                    log::error!(
                        "hosted checkout session {checkout_session_id}: captured payment \
                         {payment_id} could not settle transaction {} (now {:?}); manual review \
                         required",
                        row.id,
                        final_tx.status
                    );
                    return Ok(HostedSetupOutcome::HeldForReview { payment_method });
                }
                // Marker released only AFTER materialization succeeds: a
                // failure here leaves the Settled row sweepable for retry.
                self.on_hosted_checkout_fulfilled(
                    tenant_id,
                    checkout_session_id,
                    payment_method.id,
                    Some(payment_id),
                    Some(processed_at),
                )
                .await?;
                self.release_hosted_intent_marker(tenant_id, row).await?;
                Ok(HostedSetupOutcome::CheckoutActivated(payment_method))
            }
            CapturedPaymentResolution::HoldMismatch {
                amount_received_minor,
                remote_currency,
            } => {
                // Captured figures don't match the transaction: NEVER settle a
                // differing amount/currency — hold for review.
                log::error!(
                    "hosted checkout session {checkout_session_id}: captured payment \
                     {payment_id} reports {amount_received_minor} {remote_currency} but \
                     transaction {} expects {} {}; holding for manual review — NOT settling",
                    row.id,
                    row.amount,
                    row.currency
                );
                Ok(HostedSetupOutcome::HeldForReview { payment_method })
            }
            CapturedPaymentResolution::RecordPending => {
                // Record id + method, materialize in-flight; reconcile polls to settlement.
                self.on_hosted_checkout_fulfilled(
                    tenant_id,
                    checkout_session_id,
                    payment_method.id,
                    Some(payment_id),
                    None,
                )
                .await?;
                Ok(HostedSetupOutcome::CheckoutActivated(payment_method))
            }
        }
    }

    /// Run one provider-derived state through the shared settlement pipeline
    /// with the row locked. Terminal rows are skipped, so duplicate runs are
    /// no-ops; returns the FINAL state so callers can verify the transition applied.
    async fn consolidate_hosted_checkout_intent(
        &self,
        tenant_id: common_domain::ids::TenantId,
        transaction_id: common_domain::ids::PaymentTransactionId,
        intent: crate::domain::payment_transactions::PaymentIntent,
    ) -> StoreResult<crate::domain::PaymentTransaction> {
        let store = self.store.clone();
        self.store
            .transaction(|conn| {
                let store = store.clone();
                let intent = intent.clone();
                async move {
                    let existing = store
                        .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                        .await?;
                    store
                        .consolidate_intent_and_transaction_tx(
                            conn,
                            &Actor::System,
                            existing,
                            intent,
                        )
                        .await
                }
                .scope_boxed()
            })
            .await
    }

    /// [`Self::consolidate_hosted_checkout_intent`], additionally binding the
    /// just-saved payment method inside the same locked transaction (used by
    /// the invoice in-flow path). On Settled the pending-intent marker is
    /// released in the same transaction; declined rows keep it until close-out.
    async fn consolidate_hosted_intent(
        &self,
        tenant_id: common_domain::ids::TenantId,
        transaction_id: common_domain::ids::PaymentTransactionId,
        intent: crate::domain::payment_transactions::PaymentIntent,
        bind_payment_method: Option<CustomerPaymentMethodId>,
    ) -> StoreResult<crate::domain::PaymentTransaction> {
        use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowPatch};

        let store = self.store.clone();
        self.store
            .transaction(|conn| {
                let store = store.clone();
                let intent = intent.clone();
                async move {
                    let existing = store
                        .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                        .await?;
                    let existing = if let Some(method_id) = bind_payment_method {
                        PaymentTransactionRowPatch {
                            id: existing.id,
                            payment_method_id: Some(Some(method_id)),
                            // Redirect consumed — the customer is back.
                            next_action: Some(None),
                            ..Default::default()
                        }
                        .patch(conn, tenant_id, existing.id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into()
                    } else {
                        existing
                    };
                    let final_tx = store
                        .consolidate_intent_and_transaction_tx(
                            conn,
                            &Actor::System,
                            existing,
                            intent,
                        )
                        .await?;
                    if final_tx.status == crate::domain::PaymentStatusEnum::Settled {
                        PaymentTransactionRow::clear_pending_intent(conn, tenant_id, final_tx.id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    Ok(final_tx)
                }
                .scope_boxed()
            })
            .await
    }
}

/// How a hosted INVOICE setup settles, decided purely from the intent
/// evidence. Invariant: an in-flow intent NEVER reaches the off-session
/// charge (its hosted capture is the single charge), and an unmappable
/// captured payment is held, never re-charged or dropped.
#[derive(Debug, PartialEq)]
enum InvoiceSetupSettlement {
    /// In-flow intent with a captured payment: record and settle from it.
    RecordCapture,
    /// In-flow intent, no capture surfaced yet: report Processing — NEVER charge.
    AwaitCapture,
    /// Capture without a transaction id: unmappable money — hold for review.
    HoldUnmappedCapture,
    /// Legacy 0-amount intent: the fail-closed off-session charge.
    LegacyOffSessionCharge,
}

fn invoice_setup_settlement(in_flow: bool, has_captured_payment: bool) -> InvoiceSetupSettlement {
    match (in_flow, has_captured_payment) {
        (true, true) => InvoiceSetupSettlement::RecordCapture,
        (true, false) => InvoiceSetupSettlement::AwaitCapture,
        (false, true) => InvoiceSetupSettlement::HoldUnmappedCapture,
        (false, false) => InvoiceSetupSettlement::LegacyOffSessionCharge,
    }
}

/// How an in-flow-captured payment resolves, from the provider's
/// authoritative status. Pure so the money-path decision table is testable.
#[derive(Debug, PartialEq)]
enum CapturedPaymentResolution {
    /// Settled at the provider AND figures match: consolidate Settled, materialize Paid.
    SettleNow {
        amount_received_minor: i64,
        processed_at: chrono::NaiveDateTime,
    },
    /// Capture in flight (or the id 404s): record the id, reconcile later.
    RecordPending,
    /// Declined at the provider: consolidate Failed, offer retry.
    Declined {
        code: Option<String>,
        message: String,
    },
    /// Cancelled at the provider: consolidate Cancelled, offer retry.
    Cancelled,
    /// Settled but the amount or currency does NOT match the local
    /// transaction: never settle a differing figure — hold for manual review.
    HoldMismatch {
        amount_received_minor: i64,
        remote_currency: String,
    },
}

fn resolve_captured_payment(
    remote: crate::adapters::payment::model::RemoteTransactionStatus,
    expected_amount_minor: i64,
    expected_currency: &str,
) -> CapturedPaymentResolution {
    use crate::adapters::payment::model::RemoteTransactionStatus;
    match remote {
        RemoteTransactionStatus::Succeeded {
            amount_received_minor,
            currency,
            processed_at,
        } => {
            // Some providers report lowercase ISO codes; local rows store uppercase.
            if amount_received_minor == expected_amount_minor
                && currency.eq_ignore_ascii_case(expected_currency)
            {
                CapturedPaymentResolution::SettleNow {
                    amount_received_minor,
                    processed_at,
                }
            } else {
                CapturedPaymentResolution::HoldMismatch {
                    amount_received_minor,
                    remote_currency: currency,
                }
            }
        }
        RemoteTransactionStatus::Pending | RemoteTransactionStatus::Unknown => {
            CapturedPaymentResolution::RecordPending
        }
        RemoteTransactionStatus::Failed { code, message, .. } => {
            CapturedPaymentResolution::Declined { code, message }
        }
        RemoteTransactionStatus::Cancelled => CapturedPaymentResolution::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::payment::model::{DeclineKind, RemoteTransactionStatus};

    /// No resolution ever triggers a new charge: the hosted capture is the
    /// single charge on this path.
    #[test]
    fn captured_payment_resolution_table() {
        let processed_at = chrono::Utc::now().naive_utc();
        assert_eq!(
            resolve_captured_payment(
                RemoteTransactionStatus::Succeeded {
                    amount_received_minor: 4_200,
                    currency: "eur".into(),
                    processed_at,
                },
                4_200,
                "EUR",
            ),
            CapturedPaymentResolution::SettleNow {
                amount_received_minor: 4_200,
                processed_at,
            }
        );
        assert_eq!(
            resolve_captured_payment(RemoteTransactionStatus::Pending, 4_200, "EUR"),
            CapturedPaymentResolution::RecordPending
        );
        // A 404 on an intent-carried id is an env mismatch: record, never re-charge.
        assert_eq!(
            resolve_captured_payment(RemoteTransactionStatus::Unknown, 4_200, "EUR"),
            CapturedPaymentResolution::RecordPending
        );
        assert_eq!(
            resolve_captured_payment(
                RemoteTransactionStatus::Failed {
                    code: Some("51".into()),
                    message: "insufficient funds".into(),
                    decline_kind: DeclineKind::InsufficientFunds,
                },
                4_200,
                "EUR",
            ),
            CapturedPaymentResolution::Declined {
                code: Some("51".into()),
                message: "insufficient funds".into(),
            }
        );
        assert_eq!(
            resolve_captured_payment(RemoteTransactionStatus::Cancelled, 4_200, "EUR"),
            CapturedPaymentResolution::Cancelled
        );
    }

    /// A settled payment whose amount OR currency differs must NEVER settle;
    /// currency comparison is case-insensitive so a match is never falsely held.
    #[test]
    fn captured_payment_amount_currency_mismatch_is_held() {
        let processed_at = chrono::Utc::now().naive_utc();
        // Amount differs.
        assert_eq!(
            resolve_captured_payment(
                RemoteTransactionStatus::Succeeded {
                    amount_received_minor: 9_999,
                    currency: "eur".into(),
                    processed_at,
                },
                4_200,
                "EUR",
            ),
            CapturedPaymentResolution::HoldMismatch {
                amount_received_minor: 9_999,
                remote_currency: "eur".into(),
            }
        );
        // Currency differs.
        assert_eq!(
            resolve_captured_payment(
                RemoteTransactionStatus::Succeeded {
                    amount_received_minor: 4_200,
                    currency: "usd".into(),
                    processed_at,
                },
                4_200,
                "EUR",
            ),
            CapturedPaymentResolution::HoldMismatch {
                amount_received_minor: 4_200,
                remote_currency: "usd".into(),
            }
        );
        // Case-only difference is a MATCH, never a hold.
        assert_eq!(
            resolve_captured_payment(
                RemoteTransactionStatus::Succeeded {
                    amount_received_minor: 4_200,
                    currency: "EUR".into(),
                    processed_at,
                },
                4_200,
                "eur",
            ),
            CapturedPaymentResolution::SettleNow {
                amount_received_minor: 4_200,
                processed_at,
            }
        );
    }

    /// An in-flow intent (stamped transaction id) can NEVER select the legacy
    /// off-session charge; unmappable captured money is held, never re-charged.
    #[test]
    fn invoice_setup_settlement_never_charges_in_flow() {
        assert_eq!(
            invoice_setup_settlement(true, true),
            InvoiceSetupSettlement::RecordCapture
        );
        assert_eq!(
            invoice_setup_settlement(true, false),
            InvoiceSetupSettlement::AwaitCapture
        );
        assert_eq!(
            invoice_setup_settlement(false, true),
            InvoiceSetupSettlement::HoldUnmappedCapture
        );
        // Only the legacy 0-amount intent keeps the off-session charge.
        assert_eq!(
            invoice_setup_settlement(false, false),
            InvoiceSetupSettlement::LegacyOffSessionCharge
        );
    }
}
