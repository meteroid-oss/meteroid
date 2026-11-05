use crate::StoreResult;
use crate::adapters::payment::bridge::payment_intent_from_outcome;
use crate::adapters::payment::error::{AdoptedCharge, ConnectorError};
use crate::adapters::payment::model::{ChargeRequest, IdempotencyKey, PaymentDescriptor};
use crate::adapters::payment::{PaymentConnector, initialize_payment_connector};
use crate::domain::connectors::Connector;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::PaymentTransactionEvent;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
use crate::domain::{DeclineKind, PaymentStatusEnum as DomainPaymentStatus};
use crate::errors::StoreError;
use crate::repositories::payment_transactions::PaymentTransactionInterface;
use crate::services::InvoiceDelivery;
use crate::services::Services;
use crate::store::PgConn;
use chrono::{DateTime, NaiveDateTime, Utc};
use common_domain::ids::{
    BaseId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::enums::{InvoicePaymentStatus, PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{
    PaymentTransactionRow, PaymentTransactionRowNew, PaymentTransactionRowPatch,
};
use error_stack::{Report, ResultExt};
use scoped_futures::ScopedFutureExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Maximum time to wait for payment provider API calls.
/// This is a safety net in addition to the HTTP client timeout.
pub(in crate::services) const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

/// The provider request of an invoice charge, committed with its Pending row before it is sent,
/// so a re-send after a lost answer is byte-identical under the same idempotency key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::services) struct ChargeAttempt {
    idempotency_key: String,
    on_session: bool,
    merchant_name: String,
    invoice_number: Option<String>,
}

/// A Pending invoice charge and everything needed to send it.
pub(in crate::services) struct InvoiceCharge {
    transaction: PaymentTransactionRow,
    method: CustomerPaymentMethodRow,
    external_customer_id: String,
    connector: Connector,
    connector_impl: Box<dyn PaymentConnector>,
    attempt: ChargeAttempt,
}

pub(in crate::services) enum InvoiceChargeClaim {
    /// Another payment for the invoice is still in flight.
    InFlight(PaymentTransaction),
    /// This seeded attempt is already recorded (a redelivery after its ack was lost).
    Recorded(PaymentTransaction),
    Claimed(Box<InvoiceCharge>),
}

/// What the provider answered, sorted by what it proves about the money.
pub(in crate::services) enum ChargeAnswer {
    /// `amount` is what the provider holds: the request's, unless it adopted an earlier payment
    /// made under this key for another amount.
    Outcome {
        intent: Box<PaymentIntent>,
        amount: i64,
    },
    /// Refused before anything was created (bad request, credentials): the attempt is undone.
    Refused(Report<StoreError>),
    /// The outcome can no longer be confirmed and re-sending could charge twice (replay window
    /// over, invoice no longer payable, or a re-send refused while the first send is unknown).
    Unconfirmed,
    /// No answer (timeout, transport): the money may have moved, so the row stays Pending.
    Unknown(Report<StoreError>),
}

pub enum InvoiceChargeResult {
    Charged(PaymentTransaction, Option<PaymentNextAction>),
    /// Another payment for the invoice is still in flight; nothing was sent.
    InFlight(PaymentTransaction),
}

impl Services {
    /// Charges an invoice in one transaction, for callers whose invoice is not committed yet
    /// (checkout `FinalizeAfterPayment`, slot upgrades). A rollback after the provider took the
    /// money loses the row: prefer [`Self::charge_invoice`] whenever the invoice is committed.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn process_invoice_payment_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        customer_initiated: bool,
        on_session: bool,
        idempotency_ref: Option<String>,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        self.process_invoice_payment_at_tx(
            conn,
            tenant_id,
            invoice_id,
            payment_method_id,
            customer_initiated,
            on_session,
            idempotency_ref,
            None,
        )
        .await
    }

    /// `process_invoice_payment_tx`, with a charge the provider answered recorded as processed at
    /// `processed_at` when set: a replay's business time instead of the provider's clock.
    #[allow(clippy::too_many_arguments)]
    async fn process_invoice_payment_at_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        customer_initiated: bool,
        on_session: bool,
        idempotency_ref: Option<String>,
        processed_at: Option<NaiveDateTime>,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        let charge = match self
            .claim_invoice_charge_tx(
                conn,
                tenant_id,
                invoice_id,
                payment_method_id,
                customer_initiated,
                on_session,
                idempotency_ref,
            )
            .await?
        {
            InvoiceChargeClaim::InFlight(_) => return Err(payment_in_flight()),
            InvoiceChargeClaim::Recorded(tx) => return Ok((tx, None)),
            InvoiceChargeClaim::Claimed(charge) => charge,
        };

        let (intent, amount) = match self.send_invoice_charge(&charge).await {
            ChargeAnswer::Outcome { intent, amount } => (*intent, amount),
            ChargeAnswer::Refused(e) | ChargeAnswer::Unknown(e) => return Err(e),
            ChargeAnswer::Unconfirmed => return Err(Report::new(StoreError::PaymentProviderError)),
        };
        let intent = match processed_at {
            Some(at) if intent.status != DomainPaymentStatus::Pending => PaymentIntent {
                processed_at: Some(at),
                ..intent
            },
            _ => intent,
        };
        let next_action = intent.next_action.clone();
        let row = if amount == charge.transaction.amount {
            charge.transaction
        } else {
            record_charged_amount(
                conn,
                charge.transaction.tenant_id,
                charge.transaction.id,
                amount,
            )
            .await?
        };
        let tx = self
            .store
            .consolidate_intent_and_transaction_tx(conn, &Actor::System, row.into(), intent)
            .await?;
        Ok((tx, next_action))
    }

    /// Charges a committed invoice without holding a connection across the provider call: the
    /// Pending row (and its exact request) is committed first, the charge is sent, then its
    /// answer is recorded. An answer that never comes back leaves the row Pending for
    /// [`Self::resume_unsent_invoice_charge`] to re-send under the same key.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn charge_invoice(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        // True when a customer triggered this payment (portal), so the settled
        // payment is attributed to them; false for system auto-charges.
        customer_initiated: bool,
        on_session: bool,
        // Stable, caller-supplied seed for the provider idempotency key, so a redelivery
        // reuses the same key (and transaction id). Ignored by providers with their own seed.
        idempotency_ref: Option<String>,
        mark_processing: bool,
    ) -> StoreResult<InvoiceChargeResult> {
        let claim = self
            .store
            .transaction(|conn| {
                async move {
                    self.claim_invoice_charge_tx(
                        conn,
                        tenant_id,
                        invoice_id,
                        payment_method_id,
                        customer_initiated,
                        on_session,
                        idempotency_ref,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await?;

        let charge = match claim {
            InvoiceChargeClaim::InFlight(tx) => return Ok(InvoiceChargeResult::InFlight(tx)),
            InvoiceChargeClaim::Recorded(tx) => return Ok(InvoiceChargeResult::Charged(tx, None)),
            InvoiceChargeClaim::Claimed(charge) => charge,
        };

        let answer = self.send_invoice_charge(&charge).await;
        let (tx, next_action) = self
            .settle_invoice_charge(&charge, answer, mark_processing)
            .await?;
        Ok(InvoiceChargeResult::Charged(tx, next_action))
    }

    /// Re-sends an invoice charge whose provider answer never came back, under the idempotency
    /// key it was committed with, so a provider that already took it answers with the original.
    /// When that is no longer safe (see [`ChargeAnswer::Unconfirmed`]) the attempt is failed
    /// without dunning; a late success webhook still settles it (Failed → Settled is honoured).
    pub(in crate::services) async fn resume_unsent_invoice_charge(
        &self,
        transaction_id: PaymentTransactionId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        let row = PaymentTransactionRow::get_by_id(&mut conn, transaction_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let unsent = row.status == PaymentStatusEnum::Pending
            && row.payment_type == PaymentTypeEnum::Payment
            && row.provider_transaction_id.is_none()
            && row.invoice_id.is_some()
            && row.charge_attempt.is_some();
        if !unsent {
            return Ok(());
        }
        let charge = self.load_unsent_charge(&mut conn, &row).await;
        drop(conn);

        let age = chrono::Utc::now() - row.created_at;
        let charge = match charge {
            Ok(charge) => charge,
            Err(e) if age >= longest_charge_replay_window() => {
                log::error!("Unsent charge {transaction_id} can no longer be loaded: {e:?}");
                self.settle_charge_row(&row, ChargeAnswer::Unconfirmed, false)
                    .await?;
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        let window =
            chrono::Duration::from_std(charge.connector_impl.capabilities().charge_replay_window)
                .unwrap_or_default();
        let answer = if age >= window {
            log::warn!(
                "Charge {transaction_id} never got a provider answer within its replay window; \
                 failing it rather than risking a second charge"
            );
            ChargeAnswer::Unconfirmed
        } else if !self.still_chargeable(&charge).await? {
            log::warn!("Charge {transaction_id} is no longer chargeable; not re-sending it");
            ChargeAnswer::Unconfirmed
        } else {
            match self.send_invoice_charge(&charge).await {
                // The refusal answers this re-send only: the first one's outcome is still unknown.
                ChargeAnswer::Refused(e) => {
                    log::warn!("Re-send of charge {transaction_id} refused: {e:?}");
                    ChargeAnswer::Unconfirmed
                }
                answer => answer,
            }
        };

        self.settle_invoice_charge(&charge, answer, true).await?;
        Ok(())
    }

    async fn load_unsent_charge(
        &self,
        conn: &mut PgConn,
        row: &PaymentTransactionRow,
    ) -> StoreResult<InvoiceCharge> {
        let (Some(attempt), Some(payment_method_id)) =
            (row.charge_attempt.clone(), row.payment_method_id)
        else {
            return Err(Report::new(StoreError::InvalidArgument(
                "Unsent charge has no attempt or payment method".to_string(),
            )));
        };
        let attempt: ChargeAttempt = serde_json::from_value(attempt)
            .map_err(|e| StoreError::SerdeError("Invalid charge attempt".to_string(), e))?;
        let (method, external_customer_id, connector, connector_impl) = self
            .load_charge_context(conn, &row.tenant_id, &payment_method_id)
            .await?;
        Ok(InvoiceCharge {
            transaction: row.clone(),
            method,
            external_customer_id,
            connector,
            connector_impl,
            attempt,
        })
    }

    /// The claim's gates, re-run before a re-send: the invoice may have been paid by other means
    /// or voided, or the connector disabled, since the attempt was committed.
    async fn still_chargeable(&self, charge: &InvoiceCharge) -> StoreResult<bool> {
        let Some(invoice_id) = charge.transaction.invoice_id else {
            return Ok(true);
        };
        let tenant_id = charge.transaction.tenant_id;
        let connector = &charge.connector;
        self.store
            .transaction(|conn| {
                async move {
                    let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .invoice;
                    let payable = invoice.consolidated_into_invoice_id.is_none()
                        && matches!(
                            invoice.status,
                            diesel_models::enums::InvoiceStatusEnum::Draft
                                | diesel_models::enums::InvoiceStatusEnum::Finalized
                        )
                        && invoice.amount_due > 0;
                    if !payable {
                        return Ok(false);
                    }
                    match connector.ensure_active_locked(conn).await {
                        Ok(()) => Ok(true),
                        Err(e)
                            if matches!(e.current_context(), StoreError::ConnectorNotActive(_)) =>
                        {
                            Ok(false)
                        }
                        Err(e) => Err(e),
                    }
                }
                .scope_boxed()
            })
            .await
    }

    /// Locks the invoice, checks it can be paid, and inserts the Pending row with its exact
    /// provider request. Nothing is sent: the caller sends it once this is committed.
    #[allow(clippy::too_many_arguments)]
    async fn claim_invoice_charge_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        customer_initiated: bool,
        on_session: bool,
        idempotency_ref: Option<String>,
    ) -> StoreResult<InvoiceChargeClaim> {
        let invoice = InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // A consolidated child is billed via its parent; paying it directly would double-charge.
        if invoice.invoice.consolidated_into_invoice_id.is_some() {
            return Err(Report::new(StoreError::BillingError)
                .attach("Cannot pay an invoice merged into a consolidated parent"));
        }

        // Allow both draft and finalized invoices
        if invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Draft
            && invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Finalized
        {
            return Err(Report::new(StoreError::BillingError)
                .attach("Cannot process payment for this invoice status"));
        }

        if invoice.invoice.amount_due <= 0 {
            return Err(Report::new(StoreError::BillingError).attach("Invoice has no amount due"));
        }

        let existing_transactions =
            PaymentTransactionRow::list_by_invoice_id(conn, invoice_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        // Only one payment attempt at a time. A REFUND row in flight moves money the other way
        // and never fences a charge.
        if let Some(pending) = existing_transactions.iter().find(|tx| {
            tx.transaction.payment_type.is_collection()
                && tx.transaction.status == PaymentStatusEnum::Pending
        }) {
            return Ok(InvoiceChargeClaim::InFlight(
                pending.transaction.clone().into(),
            ));
        }

        // Calculate total of active payments (pending/ready/settled) to prevent over-payment.
        // This check, combined with SELECT FOR UPDATE on the invoice, ensures atomicity
        // in a distributed environment. Involuntary claw-backs are netted out (mirroring
        // `recompute_amount_due_from_settled_payments`) so a chargebacked settlement doesn't
        // block collecting the reopened balance; REFUND rows are not payments at all.
        let active_payment_sum: i64 = existing_transactions
            .iter()
            .filter(|tx| {
                tx.transaction.payment_type.is_collection()
                    && matches!(
                        tx.transaction.status,
                        PaymentStatusEnum::Pending
                            | PaymentStatusEnum::Ready
                            | PaymentStatusEnum::Settled
                            | PaymentStatusEnum::Refunded
                    )
            })
            .map(|tx| tx.transaction.amount - tx.transaction.amount_reversed)
            .sum();

        // Prevent payment if invoice is already fully covered
        if active_payment_sum >= invoice.invoice.total {
            return Err(Report::new(StoreError::PaymentError(format!(
                "Invoice already has sufficient payments. Total: {}, Already paid: {}",
                invoice.invoice.total, active_payment_sum
            ))));
        }

        // Prevent payment if this would exceed the invoice total
        let proposed_payment = invoice.invoice.amount_due;
        if active_payment_sum + proposed_payment > invoice.invoice.total {
            return Err(Report::new(StoreError::PaymentError(format!(
                "Payment of {} would exceed invoice total. Already paid: {}, Total: {}",
                proposed_payment, active_payment_sum, invoice.invoice.total
            ))));
        }

        let (method, external_customer_id, connector, connector_impl) = self
            .load_charge_context(conn, &tenant_id, &payment_method_id)
            .await?;
        connector.ensure_active_locked(conn).await?;

        // Committed prior attempts seed the provider key (Mollie, Stancer): a retry of an
        // accepted attempt dedupes, an attempt after a decline gets a new key. A seeded key
        // also fixes the transaction id, so a re-send carries identical metadata.
        let seed = invoice_charge_idempotency_seed(
            connector_impl.as_ref(),
            &payment_method_id,
            &invoice_id,
            existing_transactions.len(),
            idempotency_ref.as_deref(),
        );
        // Not `unwrap_or_default`: the id type's `Default` is the `Uuid::max()` sentinel.
        let transaction_id = match seed.as_deref() {
            Some(seed) => transaction_id_from_seed(seed),
            None => PaymentTransactionId::new(),
        };

        if let Some(existing) = existing_transactions
            .iter()
            .find(|tx| tx.transaction.id == transaction_id)
        {
            return Ok(InvoiceChargeClaim::Recorded(
                existing.transaction.clone().into(),
            ));
        }

        let attempt = ChargeAttempt {
            idempotency_key: invoice_charge_idempotency_key(seed.as_deref(), &transaction_id)
                .as_str()
                .to_string(),
            on_session,
            // The seller as snapshotted on the invoice.
            merchant_name: invoice
                .invoice
                .seller_details
                .get("legal_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            invoice_number: Some(invoice.invoice.invoice_number.clone()),
        };

        PaymentTransactionRowNew {
            id: transaction_id,
            tenant_id,
            invoice_id: Some(invoice_id),
            provider_transaction_id: None,
            amount: invoice.invoice.amount_due,
            currency: invoice.invoice.currency.clone(),
            payment_method_id: Some(payment_method_id),
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: customer_initiated.then_some(invoice.invoice.customer_id),
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: None,
            credit_note_id: None,
            refund_mode: None,
            reversal_reason: None,
        }
        .insert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let transaction = PaymentTransactionRowPatch {
            id: transaction_id,
            charge_attempt: Some(Some(serde_json::to_value(&attempt).map_err(|e| {
                StoreError::SerdeError("Failed to serialize charge attempt".to_string(), e)
            })?)),
            ..Default::default()
        }
        .patch(conn, tenant_id, transaction_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(InvoiceChargeClaim::Claimed(Box::new(InvoiceCharge {
            transaction,
            method,
            external_customer_id,
            connector,
            connector_impl,
            attempt,
        })))
    }

    async fn load_charge_context(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        payment_method_id: &CustomerPaymentMethodId,
    ) -> StoreResult<(
        CustomerPaymentMethodRow,
        String,
        Connector,
        Box<dyn PaymentConnector>,
    )> {
        let method = CustomerPaymentMethodRow::get_by_id(conn, tenant_id, payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let external_customer_id = connection.external_customer_id;
        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;
        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        Ok((method, external_customer_id, connector, connector_impl))
    }

    /// Sends the committed request. Holds no connection: the answer is recorded afterwards.
    async fn send_invoice_charge(&self, charge: &InvoiceCharge) -> ChargeAnswer {
        let row = &charge.transaction;
        let request = ChargeRequest {
            transaction_id: row.id,
            customer_external_id: &charge.external_customer_id,
            payment_method_external_id: &charge.method.external_payment_method_id,
            payment_method_type: charge.method.payment_method_type.clone().into(),
            amount_minor: row.amount,
            currency: &row.currency,
            idempotency_key: IdempotencyKey::new(charge.attempt.idempotency_key.clone()),
            on_session: charge.attempt.on_session,
            descriptor: Some(PaymentDescriptor {
                merchant_name: charge.attempt.merchant_name.clone(),
                invoice_number: charge.attempt.invoice_number.clone(),
            }),
            webhook_url: crate::adapters::payment::model::connector_webhook_url(
                self.store.settings.webhook_base_url(),
                &charge.connector,
            ),
        };

        let Ok(outcome) = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            charge.connector_impl.charge_off_session(request),
        )
        .await
        else {
            return ChargeAnswer::Unknown(
                Report::new(StoreError::PaymentProviderError)
                    .attach("Payment provider request timed out"),
            );
        };
        self.store
            .note_rejected_credentials(&charge.connector, &outcome)
            .await;

        let outcome = match outcome {
            Err(e) => match adopted_charge(&e) {
                Some(adopted) => Ok((adopted.outcome, adopted.amount_minor)),
                None => Err(e),
            },
            Ok(outcome) => Ok((outcome, row.amount)),
        };
        match outcome {
            Ok((outcome, amount)) => ChargeAnswer::Outcome {
                intent: Box::new(payment_intent_from_outcome(
                    outcome,
                    row.id,
                    row.tenant_id,
                    amount,
                    row.currency.clone(),
                )),
                amount,
            },
            Err(e) => {
                let unknown = matches!(e.current_context(), ConnectorError::Transport(_));
                let e = e.change_context(StoreError::PaymentProviderError);
                if unknown {
                    ChargeAnswer::Unknown(e)
                } else {
                    ChargeAnswer::Refused(e)
                }
            }
        }
    }

    /// Records the provider's answer on the committed row. A refusal removes the attempt (and is
    /// returned), an unknown answer leaves it Pending (and is returned), an expired one fails it.
    async fn settle_invoice_charge(
        &self,
        charge: &InvoiceCharge,
        answer: ChargeAnswer,
        mark_processing: bool,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        self.settle_charge_row(&charge.transaction, answer, mark_processing)
            .await
    }

    async fn settle_charge_row(
        &self,
        row: &PaymentTransactionRow,
        answer: ChargeAnswer,
        mark_processing: bool,
    ) -> StoreResult<(PaymentTransaction, Option<PaymentNextAction>)> {
        let (intent, amount) = match answer {
            ChargeAnswer::Outcome { intent, amount } => (*intent, amount),
            ChargeAnswer::Unknown(e) => return Err(e),
            ChargeAnswer::Refused(e) => {
                let mut conn = self.store.get_conn().await?;
                PaymentTransactionRow::delete_unsent_charge(&mut conn, row.id, row.tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                return Err(e);
            }
            ChargeAnswer::Unconfirmed => (unconfirmed_intent(row), row.amount),
        };
        let next_action = intent.next_action.clone();
        let (transaction_id, tenant_id) = (row.id, row.tenant_id);
        let (invoice_id, payment_method_id) = (row.invoice_id, row.payment_method_id);

        let tx = self
            .store
            .transaction(|conn| {
                async move {
                    let mut existing = self
                        .store
                        .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                        .await?;
                    if amount != existing.amount {
                        existing = record_charged_amount(conn, tenant_id, transaction_id, amount)
                            .await?
                            .into();
                    }
                    let tx = self
                        .store
                        .consolidate_intent_and_transaction_tx(
                            conn,
                            &Actor::System,
                            existing,
                            intent,
                        )
                        .await?;
                    if mark_processing
                        && let (Some(invoice_id), Some(payment_method_id)) =
                            (invoice_id, payment_method_id)
                    {
                        self.mark_processing_if_async_debit(
                            conn,
                            tenant_id,
                            invoice_id,
                            payment_method_id,
                            &tx,
                        )
                        .await?;
                    }
                    Ok(tx)
                }
                .scope_boxed()
            })
            .await?;
        Ok((tx, next_action))
    }
}

impl Services {
    /// Charges `payment_method_id` off-session at `at`, a sandbox replay's past moment, recording
    /// the gateway's answer then: a capture pays the invoice inline, a decline errors it with no
    /// dunning, and a pending charge is kept for its webhook.
    pub(in crate::services) async fn charge_invoice_at(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        at: DateTime<Utc>,
        delivery: InvoiceDelivery,
    ) -> StoreResult<PaymentTransaction> {
        if at > Utc::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "an invoice cannot be charged at a future moment".to_owned(),
            )));
        }
        self.store
            .transaction(|conn| {
                async move {
                    delivery.apply_tx(conn, tenant_id, invoice_id).await?;
                    let (transaction, _) = self
                        .process_invoice_payment_at_tx(
                            conn,
                            tenant_id,
                            invoice_id,
                            payment_method_id,
                            false,
                            false,
                            None,
                            Some(at.naive_utc()),
                        )
                        .await?;
                    match transaction.status {
                        DomainPaymentStatus::Settled => {
                            let event: PaymentTransactionEvent = transaction.clone().into();
                            self.on_invoice_payment_settled_tx(conn, &event, invoice_id, at)
                                .await?;
                        }
                        DomainPaymentStatus::Failed => {
                            InvoiceRow::apply_payment_status(
                                conn,
                                invoice_id,
                                tenant_id,
                                InvoicePaymentStatus::Errored,
                                None,
                            )
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        }
                        _ => {}
                    }
                    Ok(transaction)
                }
                .scope_boxed()
            })
            .await
    }
}

/// The payment the provider already holds under this key, when its amount differs from ours.
fn adopted_charge(error: &Report<ConnectorError>) -> Option<AdoptedCharge> {
    error
        .frames()
        .find_map(|f| f.downcast_ref::<AdoptedCharge>())
        .cloned()
}

/// The row records what the provider holds, so the invoice's amount due nets what moved.
async fn record_charged_amount(
    conn: &mut PgConn,
    tenant_id: TenantId,
    transaction_id: PaymentTransactionId,
    amount: i64,
) -> StoreResult<PaymentTransactionRow> {
    PaymentTransactionRowPatch {
        id: transaction_id,
        amount: Some(amount),
        ..Default::default()
    }
    .patch(conn, tenant_id, transaction_id)
    .await
    .map_err(Into::<Report<StoreError>>::into)
}

/// Past the longest provider replay window, a row that cannot be re-sent is safe to fail.
fn longest_charge_replay_window() -> chrono::Duration {
    chrono::Duration::days(7)
}

pub(in crate::services) fn payment_in_flight() -> Report<StoreError> {
    Report::new(StoreError::PaymentError(
        "A payment for this invoice is already being processed. Please wait for it to complete before attempting another payment.".to_string(),
    ))
}

fn unconfirmed_intent(row: &PaymentTransactionRow) -> PaymentIntent {
    PaymentIntent {
        external_id: String::new(),
        transaction_id: row.id,
        tenant_id: row.tenant_id,
        amount_requested: row.amount,
        amount_received: None,
        currency: row.currency.clone(),
        next_action: None,
        status: DomainPaymentStatus::Failed,
        last_payment_error: Some("The payment provider never confirmed this charge".to_string()),
        decline_kind: Some(DeclineKind::Unconfirmed),
        processed_at: None,
    }
}

/// The provider's committed-state seed, else the caller's (both survive rollback and pgmq
/// retry). Providers with [`PaymentOps::invoice_charge_idempotency_seed`] reuse the key across
/// retries of an accepted attempt; an attempt after a decline gets a new one.
fn invoice_charge_idempotency_seed(
    connector_impl: &dyn PaymentConnector,
    payment_method_id: &CustomerPaymentMethodId,
    invoice_id: &InvoiceId,
    prior_invoice_attempts: usize,
    idempotency_ref: Option<&str>,
) -> Option<String> {
    connector_impl
        .invoice_charge_idempotency_seed(payment_method_id, invoice_id, prior_invoice_attempts)
        .or(idempotency_ref.map(str::to_string))
}

fn invoice_charge_idempotency_key(
    seed: Option<&str>,
    transaction_id: &PaymentTransactionId,
) -> IdempotencyKey {
    match seed {
        Some(seed) => IdempotencyKey::new(format!("charge:{seed}")),
        None => IdempotencyKey::new(format!("charge:{}", transaction_id.as_base62())),
    }
}

/// Deterministic transaction id for a seeded charge (name-based, RFC 9562 v8 layout).
pub(in crate::services) fn transaction_id_from_seed(seed: &str) -> PaymentTransactionId {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("payment_transaction:{seed}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes).into_uuid().into()
}

#[cfg(test)]
mod tests {
    use super::transaction_id_from_seed;
    use crate::adapters::payment::model::IdempotencyKey;
    use crate::adapters::payment::{
        ConnectorRef, GoCardlessConnector, MollieConnector, PaymentConnector, StancerConnector,
        StripeConnector,
    };
    use crate::domain::connectors::{
        GocardlessEnvironment, GocardlessPublicData, GocardlessSensitiveData, MollieSensitiveData,
        StancerSensitiveData, StripePublicData, StripeSensitiveData,
    };
    use common_domain::ids::{
        BaseId, ConnectorId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, TenantId,
    };
    use secrecy::SecretString;

    fn invoice_charge_idempotency_key(
        connector_impl: &dyn PaymentConnector,
        method: &CustomerPaymentMethodId,
        invoice: &InvoiceId,
        attempts: usize,
        seed: Option<&str>,
        tx: &PaymentTransactionId,
    ) -> IdempotencyKey {
        let seed =
            super::invoice_charge_idempotency_seed(connector_impl, method, invoice, attempts, seed);
        super::invoice_charge_idempotency_key(seed.as_deref(), tx)
    }

    /// A key the provider already used for another amount yields that payment and its amount.
    #[test]
    fn mismatched_adoption_yields_the_provider_payment() {
        use crate::adapters::payment::error::{AdoptedCharge, ConnectorError};
        use crate::adapters::payment::model::{ChargeAcknowledged, ChargeOutcome};
        use error_stack::Report;

        let adopted = Report::new(ConnectorError::Charge("mismatch".to_string())).attach_opaque(
            AdoptedCharge {
                outcome: ChargeOutcome::Pending(ChargeAcknowledged {
                    external_id: "PM1".to_string(),
                    provider_request_id: None,
                }),
                amount_minor: 900,
            },
        );
        let found = super::adopted_charge(&adopted).expect("adopted payment");
        assert_eq!(found.amount_minor, 900);
        assert!(matches!(found.outcome, ChargeOutcome::Pending(a) if a.external_id == "PM1"));
        assert!(
            super::adopted_charge(&Report::new(ConnectorError::Charge("currency".into())))
                .is_none()
        );
    }

    /// The retry of a seeded charge carries the same transaction id, so its metadata matches.
    #[test]
    fn seeded_transaction_id_is_stable_and_seed_specific() {
        assert_eq!(
            transaction_id_from_seed("mollie-charge:a:b:0"),
            transaction_id_from_seed("mollie-charge:a:b:0")
        );
        assert_ne!(
            transaction_id_from_seed("mollie-charge:a:b:0"),
            transaction_id_from_seed("mollie-charge:a:b:1")
        );
    }

    /// The key depends on the provider and the ids, never on which connection it runs through.
    fn connector_ref() -> ConnectorRef {
        ConnectorRef {
            id: ConnectorId::new(),
            tenant_id: TenantId::new(),
            alias: "test".into(),
        }
    }

    fn mollie() -> MollieConnector {
        MollieConnector::new(
            connector_ref(),
            MollieSensitiveData {
                api_key: "test_fake".into(),
                webhook_token: "token".into(),
            },
        )
    }

    fn stancer() -> StancerConnector {
        StancerConnector::new(
            connector_ref(),
            StancerSensitiveData {
                api_secret_key: "stest_fake".into(),
            },
        )
    }

    fn stripe() -> StripeConnector {
        StripeConnector::new(
            connector_ref(),
            StripePublicData {
                api_publishable_key: "pk_test_fake".into(),
                account_id: Some("acct_fake".into()),
                webhook_events_fingerprint: None,
            },
            StripeSensitiveData {
                api_secret_key: "sk_test_fake".into(),
                webhook_secret: SecretString::from(String::new()),
                webhook_endpoint_id: None,
            },
        )
    }

    fn gocardless() -> GoCardlessConnector {
        GoCardlessConnector::new(
            connector_ref(),
            GocardlessPublicData {
                creditor_id: None,
                environment: GocardlessEnvironment::Sandbox,
            },
            GocardlessSensitiveData {
                access_token: "fake".into(),
                webhook_secret: "fake".into(),
            },
        )
    }

    /// A rolled-back attempt retried with a new transaction id reuses the key, so Mollie replays
    /// it.
    #[test]
    fn mollie_charge_key_survives_a_rolled_back_attempt() {
        let (method, invoice) = (CustomerPaymentMethodId::new(), InvoiceId::new());
        let key = |attempts, seed: Option<&str>| {
            invoice_charge_idempotency_key(
                &mollie(),
                &method,
                &invoice,
                attempts,
                seed,
                &PaymentTransactionId::new(),
            )
            .as_str()
            .to_string()
        };
        let first = key(0, None);
        assert_eq!(first, key(0, None));
        assert_eq!(first, key(0, Some("gc-charge:mdt_1:inv")));
        assert_ne!(
            first,
            key(1, None),
            "a new committed attempt gets a fresh key"
        );

        let stancer = invoice_charge_idempotency_key(
            &stancer(),
            &method,
            &invoice,
            0,
            None,
            &PaymentTransactionId::new(),
        );
        assert_ne!(first, stancer.as_str(), "providers never share a key");
    }

    /// Stancer's key must stay byte-identical for in-flight retries.
    #[test]
    fn stancer_charge_key_format_is_unchanged() {
        let (method, invoice) = (CustomerPaymentMethodId::new(), InvoiceId::new());
        let key = invoice_charge_idempotency_key(
            &stancer(),
            &method,
            &invoice,
            2,
            Some("ignored"),
            &PaymentTransactionId::new(),
        );
        assert_eq!(
            key.as_str(),
            format!(
                "charge:stancer-charge:{}:{}:2",
                method.as_base62(),
                invoice.as_base62()
            )
        );
    }

    /// Providers without a committed seed use the caller's seed, else the transaction id.
    #[test]
    fn other_providers_keep_seed_or_transaction_key() {
        let (method, invoice, tx) = (
            CustomerPaymentMethodId::new(),
            InvoiceId::new(),
            PaymentTransactionId::new(),
        );
        let impls: [Box<dyn PaymentConnector>; 2] = [Box::new(stripe()), Box::new(gocardless())];
        for connector_impl in impls {
            let key = |seed| {
                invoice_charge_idempotency_key(
                    connector_impl.as_ref(),
                    &method,
                    &invoice,
                    0,
                    seed,
                    &tx,
                )
            };
            assert_eq!(key(None).as_str(), format!("charge:{}", tx.as_base62()));
            assert_eq!(key(Some("seed")).as_str(), "charge:seed");
        }
    }
}
