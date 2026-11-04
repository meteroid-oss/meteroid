use crate::StoreResult;
use crate::adapters::payment::bridge::payment_intent_from_outcome;
use crate::adapters::payment::error::{AdoptedCharge, ConnectorError};
use crate::adapters::payment::model::{
    ChargeOutcome, ChargeRequest, IdempotencyKey, PaymentDescriptor,
};
use crate::adapters::payment::{PaymentConnector, initialize_payment_connector};
use crate::domain::PaymentStatusEnum as DomainPaymentStatus;
use crate::domain::connectors::Connector;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::PaymentTransactionEvent;
use crate::domain::payment_transactions::{PaymentIntent, PaymentNextAction, PaymentTransaction};
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
use std::time::Duration;

/// Maximum time to wait for payment provider API calls.
/// This is a safety net in addition to the HTTP client timeout.
pub(in crate::services) const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

impl Services {
    /// Creates a payment intent and the associated payment transaction.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn process_invoice_payment_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        payment_method_id: CustomerPaymentMethodId,
        // True when a customer triggered this payment (portal), so the settled
        // payment is attributed to them; false for system auto-charges.
        customer_initiated: bool,
        on_session: bool,
        // Stable, caller-supplied seed for the provider idempotency key. When
        // set, a retry that mints a fresh transaction id still reuses the SAME
        // provider key, so the provider dedupes instead of double-charging. Only
        // the webhook-driven off-session invoice charge (which can be retried by
        // pgmq) passes this; interactive callers pass None (per-transaction key).
        // Ignored for Stancer connections, which always derive a stable
        // invoice-scoped key — see `create_payment_intent`.
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
        // Get the invoice
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

        // Check for existing transactions that would prevent a new payment
        let existing_transactions =
            PaymentTransactionRow::list_by_invoice_id(conn, invoice_id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        // Check for pending transactions - only one payment attempt at a time.
        // A REFUND row in flight moves money the other way and never fences a charge.
        let has_pending_transaction = existing_transactions.iter().any(|tx| {
            tx.transaction.payment_type.is_collection()
                && tx.transaction.status == PaymentStatusEnum::Pending
        });

        if has_pending_transaction {
            return Err(Report::new(StoreError::PaymentError(
                "A payment for this invoice is already being processed. Please wait for it to complete before attempting another payment.".to_string()
            )));
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

        // Committed prior attempts (never the uncommitted row inserted below):
        // seeds the per-attempt component of the Stancer idempotency key in
        // `create_payment_intent`. Rollback-stable — a retry after a rolled-back
        // charge recomputes the same value and dedupes at the provider.
        let prior_invoice_attempts = existing_transactions.len();

        // Persist the Pending row BEFORE the external charge so the provider
        // idempotency key `charge:{id}` is derived from a row that already exists:
        // a retry that reuses this id can never double-charge.
        //
        // Residual window: this insert shares the surrounding DB transaction. When
        // the invoice is created in that same (uncommitted) transaction — the
        // checkout `FinalizeAfterPayment` path — a fresh connection can't see it
        // (FK invisible), so the row genuinely can't be committed ahead of the
        // charge. If the surrounding transaction later rolls back after the charge
        // succeeded, the row is lost; recovery is via the provider webhook /
        // reconciliation (which re-drive from the provider's own record). A retry
        // through this function mints a NEW id only if the row did not survive —
        // that is the uncovered window, bounded to a crash between charge and commit.
        // For Stancer (no webhook, reconcile polls local rows only) that window is
        // closed instead by the invoice-derived key in `create_payment_intent`.
        let charge = self
            .prepare_invoice_charge(
                conn,
                &tenant_id,
                &invoice_id,
                &payment_method_id,
                idempotency_ref.as_deref(),
                prior_invoice_attempts,
            )
            .await?;

        // Same seed as a committed attempt (e.g. a redelivery after its ack was lost).
        if let Some(existing) = existing_transactions
            .iter()
            .find(|tx| tx.transaction.id == charge.transaction_id)
        {
            return Ok((existing.transaction.clone().into(), None));
        }

        let transaction = PaymentTransactionRowNew {
            id: charge.transaction_id,
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
        };

        let inserted_transaction = transaction
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Create payment intent with payment provider
        let (payment_intent, charged_amount) = self
            .create_payment_intent(
                charge,
                &tenant_id,
                inserted_transaction.amount as u64,
                inserted_transaction.currency.clone(),
                on_session,
                Some(PaymentDescriptor {
                    // The seller as snapshotted on the invoice.
                    merchant_name: invoice
                        .invoice
                        .seller_details
                        .get("legal_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    invoice_number: Some(invoice.invoice.invoice_number.clone()),
                }),
            )
            .await?;

        let payment_intent = match processed_at {
            Some(at) if payment_intent.status != DomainPaymentStatus::Pending => PaymentIntent {
                processed_at: Some(at),
                ..payment_intent
            },
            _ => payment_intent,
        };

        // Transient next_action (carries the client secret) — surfaced to the
        // on-session caller so the portal can complete 3DS; never persisted.
        let next_action = payment_intent.next_action.clone();

        let inserted_transaction = if charged_amount == inserted_transaction.amount {
            inserted_transaction
        } else {
            record_charged_amount(conn, &inserted_transaction, charged_amount).await?
        };

        // Consolidate the transaction
        let tx = self
            .store
            .consolidate_intent_and_transaction_tx(
                conn,
                &Actor::System,
                inserted_transaction.into(),
                payment_intent,
            )
            .await?;

        Ok((tx, next_action))
    }

    /// Loads the charge's provider context and fixes its idempotency key. A seeded key
    /// (stable across retries) also fixes the transaction id, so a retry after a rollback
    /// resends identical parameters (`meteroid.transaction_id` metadata included) and the
    /// provider replays the original charge instead of rejecting a parameter mismatch.
    async fn prepare_invoice_charge(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        invoice_id: &InvoiceId,
        payment_method_id: &CustomerPaymentMethodId,
        idempotency_ref: Option<&str>,
        prior_invoice_attempts: usize,
    ) -> StoreResult<PreparedInvoiceCharge> {
        let method = CustomerPaymentMethodRow::get_by_id(conn, tenant_id, payment_method_id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, &method.connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let external_customer_id = connection.external_customer_id;
        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;
        connector.ensure_active_locked(conn).await?;

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let seed = invoice_charge_idempotency_seed(
            connector_impl.as_ref(),
            payment_method_id,
            invoice_id,
            prior_invoice_attempts,
            idempotency_ref,
        );
        // Not `unwrap_or_default`: the id type's `Default` is the `Uuid::max()` sentinel.
        let transaction_id = match seed.as_deref() {
            Some(seed) => transaction_id_from_seed(seed),
            None => PaymentTransactionId::new(),
        };
        let idempotency_key = invoice_charge_idempotency_key(seed.as_deref(), &transaction_id);

        Ok(PreparedInvoiceCharge {
            method,
            external_customer_id,
            connector,
            connector_impl,
            transaction_id,
            idempotency_key,
        })
    }

    async fn create_payment_intent(
        &self,
        charge: PreparedInvoiceCharge,
        tenant_id: &TenantId,
        amount: u64,
        currency: String,
        on_session: bool,
        descriptor: Option<PaymentDescriptor>,
    ) -> StoreResult<(PaymentIntent, i64)> {
        let PreparedInvoiceCharge {
            method,
            external_customer_id,
            connector,
            connector_impl,
            transaction_id,
            idempotency_key,
        } = charge;
        let transaction_id = &transaction_id;

        let request = ChargeRequest {
            transaction_id: *transaction_id,
            customer_external_id: &external_customer_id,
            payment_method_external_id: &method.external_payment_method_id,
            payment_method_type: method.payment_method_type.clone().into(),
            amount_minor: amount as i64,
            currency: &currency,
            idempotency_key,
            on_session,
            descriptor,
            webhook_url: crate::adapters::payment::model::connector_webhook_url(
                self.store.settings.webhook_base_url(),
                &connector,
            ),
        };

        let outcome = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.charge_off_session(request),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?;
        self.store
            .note_rejected_credentials(&connector, &outcome)
            .await;
        let (outcome, amount) = adopted_or_outcome(outcome, amount as i64)?;

        Ok((
            payment_intent_from_outcome(outcome, *transaction_id, *tenant_id, amount, currency),
            amount,
        ))
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

/// The provider's answer, or the payment it already holds under this key when that one's amount
/// differs (see [`AdoptedCharge`]), with the amount actually charged.
pub(in crate::services) fn adopted_or_outcome(
    outcome: Result<ChargeOutcome, Report<ConnectorError>>,
    requested_amount: i64,
) -> StoreResult<(ChargeOutcome, i64)> {
    match outcome {
        Ok(outcome) => Ok((outcome, requested_amount)),
        Err(e) => match e
            .frames()
            .find_map(|f| f.downcast_ref::<AdoptedCharge>())
            .cloned()
        {
            Some(adopted) => Ok((adopted.outcome, adopted.amount_minor)),
            None => Err(e.change_context(StoreError::PaymentProviderError)),
        },
    }
}

/// The row records what the provider holds, so the invoice's amount due nets what moved.
pub(in crate::services) async fn record_charged_amount(
    conn: &mut PgConn,
    row: &PaymentTransactionRow,
    amount: i64,
) -> StoreResult<PaymentTransactionRow> {
    PaymentTransactionRowPatch {
        id: row.id,
        amount: Some(amount),
        ..Default::default()
    }
    .patch(conn, row.tenant_id, row.id)
    .await
    .map_err(Into::<Report<StoreError>>::into)
}

struct PreparedInvoiceCharge {
    method: CustomerPaymentMethodRow,
    external_customer_id: String,
    connector: Connector,
    connector_impl: Box<dyn PaymentConnector>,
    transaction_id: PaymentTransactionId,
    idempotency_key: IdempotencyKey,
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

    /// A key the provider already used for another amount records that payment, at its amount.
    #[test]
    fn mismatched_adoption_records_the_provider_payment() {
        use crate::adapters::payment::error::{AdoptedCharge, ConnectorError};
        use crate::adapters::payment::model::{ChargeAcknowledged, ChargeOutcome};
        use error_stack::Report;

        let existing = ChargeOutcome::Pending(ChargeAcknowledged {
            external_id: "PM1".to_string(),
            provider_request_id: None,
        });
        let adopted = Report::new(ConnectorError::Charge("mismatch".to_string())).attach_opaque(
            AdoptedCharge {
                outcome: existing,
                amount_minor: 900,
            },
        );
        let (outcome, amount) = super::adopted_or_outcome(Err(adopted), 1000).unwrap();
        assert_eq!(amount, 900);
        assert!(matches!(outcome, ChargeOutcome::Pending(a) if a.external_id == "PM1"));

        let refused = Report::new(ConnectorError::Charge("currency".to_string()));
        assert!(super::adopted_or_outcome(Err(refused), 1000).is_err());
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
