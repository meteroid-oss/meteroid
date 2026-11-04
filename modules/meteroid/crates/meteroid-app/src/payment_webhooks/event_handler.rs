//! Provider-agnostic dispatcher for [`NormalizedWebhookEvent`].
//!
//! The webhook router verifies the signature and parses the payload via the
//! per-provider adapter ([`PaymentConnector::parse_event`]), then hands the
//! normalized event to this dispatcher. The dispatcher only sees the closed
//! enum [`NormalizedEventKind`] — no Stripe/GoCardless/Adyen types leak in.

use crate::errors;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
    InvoiceId, PaymentTransactionId, TenantId,
};
use error_stack::{Report, ResultExt};
use meteroid_store::adapters::payment::bridge::payment_intent_from_event;
use meteroid_store::adapters::payment::error::HostedSetupFailed;
use meteroid_store::adapters::payment::events::{NormalizedEventKind, NormalizedWebhookEvent};
use meteroid_store::adapters::payment::model::SetupPurpose;
use meteroid_store::adapters::payment::{PaymentConnector, RemoteRefundStatus};
use meteroid_store::domain::CustomerPaymentMethodNew;
use meteroid_store::domain::ObservedRefundStatus;
use meteroid_store::domain::connectors::Connector;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::enums::{PaymentStatusEnum, PaymentTypeEnum, ReversalKind};
use meteroid_store::domain::payment_transactions::PaymentTransaction;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::payment_transactions::{
    ObservedRefund, ObservedRefunds, PaymentTransactionInterface, RefundSettlement,
    RefundSettlementStatus, ReversalAmount, TransactionReinstatement, TransactionReversal,
};
use meteroid_store::{Services, Store};
use scoped_futures::ScopedFutureExt;

/// Dispatch a verified, parsed webhook event onto the store. Runs from the pgmq
/// `webhook_in` worker: returning `Err` makes the worker retry the message and
/// eventually dead-letter it (alertable), so only return `Ok` when the event is
/// genuinely handled or a safe duplicate — never to swallow a lost money event.
pub async fn handle_normalized_event(
    event: NormalizedWebhookEvent,
    connector: &Connector,
    connector_impl: &dyn PaymentConnector,
    store: Store,
    services: &Services,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    log::info!(
        "Processing webhook event {} (type={:?})",
        event.provider_event_id,
        event.provider_event_type,
    );

    // Cross-tenant delivery guard: a provider account shared by two tenants fans
    // every event out to both endpoints, but only the owner has the records. Skip
    // only on a POSITIVELY-named foreign tenant; missing/unparseable → fall through.
    if let Some(claimed) = event.owner_tenant_id.as_deref()
        && let Ok(owner) = TenantId::parse_base62(claimed)
        && owner != connector.tenant_id
    {
        log::info!(
            "Skipping webhook event {} — it belongs to tenant {}, delivered to tenant {} on a shared provider account",
            event.provider_event_id,
            claimed,
            connector.tenant_id.as_base62(),
        );
        return Ok(());
    }

    match &event.kind {
        NormalizedEventKind::PaymentMethodAttached(e) => {
            handle_payment_method_attached(e, connector, connector_impl, &store, services).await
        }
        NormalizedEventKind::MandateSetupCompleted { provider_intent_id } => {
            handle_mandate_setup_completed(
                provider_intent_id,
                connector,
                connector_impl,
                &store,
                services,
            )
            .await
        }
        NormalizedEventKind::PaymentSucceeded(_)
        | NormalizedEventKind::PaymentFailed(_)
        | NormalizedEventKind::PaymentPending(_) => {
            handle_payment_state_change(&event, connector, &store).await
        }
        NormalizedEventKind::PaymentRequiresAction(e) => {
            handle_payment_requires_action(e, connector, &store).await
        }
        NormalizedEventKind::PaymentMethodUpdated(e) => {
            handle_payment_method_updated(e, connector, &store).await
        }
        NormalizedEventKind::PaymentMethodDetached(e) => {
            handle_payment_method_detached(e, connector, &store).await
        }
        NormalizedEventKind::PaymentMethodExpiring(e) => {
            // The card will expire; the customer must update it. Surfaced for
            // the notification hook — no DB change.
            log::warn!(
                "Payment method {} is expiring (expires {}); customer should update it",
                e.external_payment_method_id,
                e.expires_at
            );
            Ok(())
        }
        // ── voluntary refunds ─────────────────────────────────────────────
        // Stripe `charge.refunded` (cumulative, partial-aware), Mollie, mock: money the
        // merchant gave back. Recorded as REFUND rows against the payment; the invoice keeps
        // its balance, the Refund credit note is what reduces it.
        NormalizedEventKind::PaymentRefunded(e) => {
            handle_observed_refunds(
                &store,
                connector,
                e.external_transaction_id.clone(),
                None,
                Some(e.amount_refunded_minor),
                observed_refunds_from_event(e),
                event.occurred_at,
            )
            .await
        }
        // Provider-side (dashboard) refund whose event carries no amounts
        // (GoCardless): resolve the refund and its parent from the provider first.
        NormalizedEventKind::RefundObserved { external_refund_id } => {
            handle_refund_observed(
                external_refund_id,
                connector,
                connector_impl,
                &store,
                event.occurred_at,
            )
            .await
        }
        // Stripe `refund.updated` / `charge.refund.updated`: a refund we issued moved.
        NormalizedEventKind::RefundStatusChanged(e) => {
            handle_refund_status_change(e, connector, &store, event.occurred_at).await
        }
        // ── involuntary reversals ─────────────────────────────────────────
        // GoCardless `charged_back` (the payer's claim: the invoice stays paid) or
        // `late_failure_settled` (a failed collection: it reopens), full amount.
        NormalizedEventKind::PaymentReversed(e) => {
            handle_reversal(
                &store,
                connector,
                e.external_transaction_id.clone(),
                e.meteroid_transaction_id.clone(),
                ReversalAmount::Full,
                e.kind,
                e.disputed,
                Some(e.cause.clone().unwrap_or_else(|| e.reason.clone())),
                Some(event.provider_event_id.clone()),
                event.occurred_at,
            )
            .await
        }
        // GoCardless `chargeback_cancelled`: the bank returned the full clawed-back amount. Its
        // event names no dispute its charge-back shares, so the reversal mark guards redelivery.
        NormalizedEventKind::PaymentReinstated(e) => {
            handle_reinstatement(
                &store,
                connector,
                e.external_transaction_id.clone(),
                e.meteroid_transaction_id.clone(),
                None,
                e.reason.clone(),
                None,
                event.occurred_at,
            )
            .await
        }
        // Stripe dispute / Mollie chargeback: funds actually withdrawn. Partial disputes withdraw
        // only their amount. A bank return reported as a dispute is a failed collection.
        NormalizedEventKind::DisputeFundsWithdrawn(e) => {
            handle_reversal(
                &store,
                connector,
                e.external_transaction_id.clone(),
                None,
                ReversalAmount::Incremental(e.amount_minor),
                e.failure.unwrap_or(ReversalKind::Dispute),
                e.failure.is_none(),
                Some(format!(
                    "dispute.{}",
                    e.reason.as_deref().unwrap_or("funds_withdrawn")
                )),
                Some(e.external_dispute_id.clone()),
                event.occurred_at,
            )
            .await
        }
        // Dispute resolved in the merchant's favor: the withdrawn amount came back.
        NormalizedEventKind::DisputeFundsReinstated(e) => {
            handle_reinstatement(
                &store,
                connector,
                e.external_transaction_id.clone(),
                None,
                Some(e.amount_minor),
                format!(
                    "dispute.{}",
                    e.reason.as_deref().unwrap_or("funds_reinstated")
                ),
                Some(e.external_dispute_id.clone()),
                event.occurred_at,
            )
            .await
        }
        // Dispute opened: funds not moved yet. Finance-alert, but no money change.
        NormalizedEventKind::DisputeOpened(e) => {
            log::error!(
                "Dispute {} opened on charge {} ({} {}, reason {:?}); funds not yet withdrawn — awaiting resolution",
                e.external_dispute_id,
                e.external_transaction_id,
                e.amount_minor,
                e.currency,
                e.reason,
            );
            Ok(())
        }
        NormalizedEventKind::Acknowledged { reason } => {
            log::debug!(
                "Acknowledged unhandled event {}: {}",
                event.provider_event_id,
                reason
            );
            Ok(())
        }
        // Payloadless notification (Mollie): a transient re-read failure propagates so pgmq
        // retries.
        NormalizedEventKind::ResourceChanged { resource_ref } => {
            let resolved = connector_impl
                .resolve_resource_change(resource_ref)
                .await
                .change_context(errors::AdapterWebhookError::ProviderError)?;
            if resolved
                .iter()
                .any(|e| matches!(e.kind, NormalizedEventKind::ResourceChanged { .. }))
            {
                return Err(Report::new(
                    errors::AdapterWebhookError::EventTypeNotSupported(
                        "resolver returned a nested ResourceChanged".to_string(),
                    ),
                ));
            }
            // Run every event even if one fails (a mandate update must not block the payment
            // state). Handlers are idempotent, so a retry safely replays all.
            let mut failure: Option<Report<errors::AdapterWebhookError>> = None;
            for resolved_event in resolved {
                let event_id = resolved_event.provider_event_id.clone();
                if let Err(err) = Box::pin(handle_normalized_event(
                    resolved_event,
                    connector,
                    connector_impl,
                    store.clone(),
                    services,
                ))
                .await
                {
                    log::warn!("resolved event {event_id} failed: {err:?}");
                    // Prefer a retryable failure over a terminal one.
                    let keep_existing = failure.as_ref().is_some_and(|f| {
                        f.current_context().is_transient() || !err.current_context().is_transient()
                    });
                    if !keep_existing {
                        failure = Some(err);
                    }
                }
            }
            failure.map_or(Ok(()), Err)
        }
        // Remaining kinds (dispute won/lost) are outcome notifications only:
        // the money movement arrives as funds_withdrawn / funds_reinstated,
        // handled above — log and ack so the provider stops retrying.
        other => {
            log::info!(
                "Webhook event kind not yet handled ({:?}); event_id={}",
                std::mem::discriminant(other),
                event.provider_event_id
            );
            Ok(())
        }
    }
}

/// SetupIntent-equivalent (Stripe `setup_intent.succeeded`): a method was
/// attached via the embedded flow. Fetch the canonical snapshot, fold in the ids
/// the event already carries (Stripe puts them on the event), then persist.
async fn handle_payment_method_attached(
    e: &meteroid_store::adapters::payment::events::PaymentMethodAttachedEvent,
    connector: &Connector,
    connector_impl: &dyn PaymentConnector,
    store: &Store,
    services: &Services,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let mut snapshot = connector_impl
        .fetch_payment_method(&e.external_payment_method_id, &e.external_customer_id)
        .await
        .change_context(errors::AdapterWebhookError::ProviderError)?;

    // Stripe carries our ids on the event; the fetched resource doesn't. Prefer
    // the event's, falling back to whatever the resource had.
    if snapshot.meteroid_connection_id.is_none() {
        snapshot.meteroid_connection_id = e.meteroid_connection_id.clone();
    }
    if snapshot.meteroid_customer_id.is_none() {
        snapshot.meteroid_customer_id = e.meteroid_customer_id.clone();
    }

    attach_payment_method_from_snapshot(snapshot, e.purpose, connector, store, services).await
}

/// GoCardless `billing_requests.fulfilled`: the hosted mandate setup completed.
/// Finalize by reading the Billing Request (recovers our ids + the created
/// mandate, with no reliance on GoCardless propagating metadata onto the
/// mandate), then persist + charge via the same path as an attached method.
async fn handle_mandate_setup_completed(
    provider_intent_id: &str,
    connector: &Connector,
    connector_impl: &dyn PaymentConnector,
    store: &Store,
    services: &Services,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let snapshot = match connector_impl
        .complete_mandate_setup(provider_intent_id)
        .await
    {
        Ok(snapshot) => snapshot,
        // Retrying won't fix it (e.g. invalid mandate): ack and log instead of dead-lettering.
        Err(report)
            if report
                .frames()
                .any(|f| f.downcast_ref::<HostedSetupFailed>().is_some()) =>
        {
            log::error!(
                "hosted setup {provider_intent_id} failed terminally; no method attached: {report:?}"
            );
            return Ok(());
        }
        Err(report) => {
            return Err(report.change_context(errors::AdapterWebhookError::ProviderError));
        }
    };

    attach_payment_method_from_snapshot(snapshot, None, connector, store, services).await
}

/// Persist a payment-method snapshot for its customer, set it as current, and —
/// when the snapshot names an invoice (hosted-redirect providers can only charge
/// once the mandate exists) — charge that invoice.
///
/// The method is saved first and unconditionally. The charge then: acks on a
/// terminal outcome (already paid/pending/over-pay — a duplicate delivery), and
/// returns Err on a transient one so pgmq retries. Retry is double-charge-safe
/// because the charge uses a stable (mandate, invoice) idempotency seed, and the
/// over-payment / pending guards backstop duplicate deliveries and cross-method
/// races.
async fn attach_payment_method_from_snapshot(
    snapshot: meteroid_store::adapters::payment::model::PaymentMethodSnapshot,
    // What an embedded setup intent was created for; a hosted one names its invoice or checkout.
    purpose: Option<SetupPurpose>,
    connector: &Connector,
    store: &Store,
    services: &Services,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let connection_id_str = snapshot.meteroid_connection_id.as_deref().ok_or_else(|| {
        Report::new(errors::AdapterWebhookError::MissingMetadata(
            "meteroid.connection_id".to_string(),
        ))
    })?;
    let connection_id = CustomerConnectionId::parse_base62(connection_id_str)
        .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

    let customer_id_str = snapshot.meteroid_customer_id.as_deref().ok_or_else(|| {
        Report::new(errors::AdapterWebhookError::MissingMetadata(
            "meteroid.customer_id".to_string(),
        ))
    })?;
    let customer_id = CustomerId::parse_base62(customer_id_str)
        .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

    // Cross-tenant / hijack defense: verify the connection belongs to this
    // connector's tenant and is owned by the named customer.
    use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
    // The store can't distinguish a genuinely-missing connection from a
    // transient DB failure, so keep this retryable: a truly-missing connection
    // exhausts pgmq retries and dead-letters (visible), while a DB blip must
    // never permanently drop a mandate-setup/charge event.
    let connection = store
        .get_connection_by_id(&connector.tenant_id, &connection_id)
        .await
        .change_context(errors::AdapterWebhookError::StoreError)
        .attach("failed to load the connection referenced by webhook metadata")?;
    if connection.customer_id != customer_id {
        return Err(Report::new(errors::AdapterWebhookError::InvalidMetadata)
            .attach("webhook connection_id / customer_id pair is inconsistent"));
    }

    let invoice_to_charge = snapshot.meteroid_invoice_id.clone();
    // Invoice paid on the hosted page (Mollie): the first payment is the invoice payment.
    let in_flow_invoice_transaction = snapshot.meteroid_transaction_id.clone();
    let in_flow_captured_payment = snapshot.payment_request_payment.clone();
    // A combined mandate+payment CHECKOUT Billing Request: the provider already
    // created the first payment, so instead of charging an invoice we materialize
    // the subscription in-flight against the pre-created checkout transaction.
    let checkout_session_to_complete = snapshot.meteroid_checkout_session_id.clone();
    let checkout_payment_id = snapshot.payment_request_payment.clone();
    // Stable across pgmq retries of this same webhook (mandate id + invoice are
    // both fixed for a given fulfillment) and unique to this logical charge — the
    // provider-idempotency seed that makes the charge safe to retry.
    let mandate_ref = snapshot.external_payment_method_id.clone();
    let pays = invoice_to_charge.is_some()
        || checkout_session_to_complete.is_some()
        || purpose == Some(SetupPurpose::Payment);

    let payment_method = store
        .upsert_payment_method(CustomerPaymentMethodNew {
            id: CustomerPaymentMethodId::new(),
            tenant_id: connector.tenant_id,
            customer_id,
            connection_id,
            external_payment_method_id: snapshot.external_payment_method_id,
            payment_method_type: snapshot.payment_method_type,
            account_number_hint: snapshot.account_number_hint,
            card_brand: snapshot.card_brand,
            card_last4: snapshot.card_last4,
            card_exp_month: snapshot.card_exp_month,
            card_exp_year: snapshot.card_exp_year,
            fingerprint: snapshot.fingerprint,
            saved_for_payment: pays,
        })
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;

    // A method set up on its own is the new default; one that pays waits for its payment to
    // settle. A setup intent from an older version records no purpose: it is the default.
    let adoption = match (pays, purpose) {
        (false, _) => Some(meteroid_store::services::DefaultMethodAdoption::Replace),
        (true, Some(SetupPurpose::Payment)) => {
            Some(meteroid_store::services::DefaultMethodAdoption::IfNone)
        }
        (true, _) => None,
    };
    if let Some(adoption) = adoption {
        services
            .adopt_default_payment_method(
                meteroid_store::domain::entity_activity::Actor::Customer { id: customer_id },
                connector.tenant_id,
                customer_id,
                payment_method.id,
                adoption,
            )
            .await
            .change_context(errors::AdapterWebhookError::StoreError)?;
    }

    if let (Some(invoice_id_str), Some(_)) = (&invoice_to_charge, &in_flow_invoice_transaction) {
        match services
            .settle_hosted_invoice_capture(
                connector.tenant_id,
                customer_id,
                connector,
                payment_method,
                invoice_id_str,
                in_flow_captured_payment,
                in_flow_invoice_transaction,
            )
            .await
        {
            Ok(outcome) => log::info!(
                "mandate attached; in-flow invoice {invoice_id_str} settlement: {outcome:?}"
            ),
            Err(e) => {
                log::warn!(
                    "mandate attached but in-flow invoice {invoice_id_str} settlement failed; retrying via pgmq: {e:?}"
                );
                return Err(e).change_context(errors::AdapterWebhookError::StoreError);
            }
        }
    } else if let Some(invoice_id_str) = invoice_to_charge {
        let invoice_id = match InvoiceId::parse_base62(&invoice_id_str) {
            Ok(id) => id,
            Err(_) => {
                log::error!("mandate metadata carried an unparseable invoice id: {invoice_id_str}");
                return Ok(());
            }
        };

        // Defense-in-depth: the invoice id was read back from provider metadata,
        // which has no auth context. Re-verify it belongs to this customer before
        // charging so stale/tampered metadata can never move money across the
        // customer boundary. (The API boundary validates it too.)
        let invoice = store
            .get_invoice_by_id(connector.tenant_id, invoice_id)
            .await
            .change_context(errors::AdapterWebhookError::StoreError)?;
        if invoice.customer_id != customer_id {
            log::error!(
                "refusing to charge invoice {} for mandate {}: invoice customer {} != mandate customer {}",
                invoice_id,
                payment_method.id,
                invoice.customer_id.as_base62(),
                customer_id.as_base62(),
            );
            return Ok(());
        }

        // Seed the provider idempotency key with (mandate, invoice) so a pgmq
        // retry of this webhook reuses the same key and the provider dedupes,
        // instead of the default per-transaction key that changes each attempt.
        let idempotency_ref = format!("gc-charge:{mandate_ref}:{}", invoice_id.as_base62());

        match services
            .complete_invoice_payment(
                connector.tenant_id,
                invoice_id,
                payment_method.id,
                false,
                Some(idempotency_ref),
            )
            .await
        {
            Ok(_) => log::info!(
                "mandate {} attached; invoice {} charge initiated",
                payment_method.id,
                invoice_id
            ),
            // Terminal — expected on a duplicate `fulfilled` delivery: a payment
            // is already pending / sufficient / would over-pay (PaymentError), or
            // the invoice is non-payable — already paid, wrong status, nothing
            // due, consolidated child (BillingError). Nothing to recover; ack.
            Err(e)
                if matches!(
                    e.current_context(),
                    StoreError::PaymentError(_)
                        | StoreError::BillingError
                        | StoreError::ConnectorNotActive(_)
                ) =>
            {
                log::info!(
                    "mandate {} attached; invoice {} not charged (already paid/pending/non-payable): {e:?}",
                    payment_method.id,
                    invoice_id
                );
            }
            // Transient (provider timeout/5xx, DB error). Return Err so pgmq
            // retries the whole handler. Safe now that the charge uses the stable
            // `idempotency_ref` above: if the provider already processed the first
            // attempt, the retry dedupes to that same payment rather than creating
            // a second. Upsert/patch are idempotent; over-payment guard backstops.
            Err(e) => {
                log::warn!(
                    "mandate {} attached but invoice {} charge failed transiently; retrying via pgmq: {e:?}",
                    payment_method.id,
                    invoice_id
                );
                return Err(e).change_context(errors::AdapterWebhookError::StoreError);
            }
        }
    } else if let Some(session_id_str) = checkout_session_to_complete {
        // Combined mandate+payment checkout: the provider created the first
        // payment as part of the hosted flow. Materialize the subscription
        // in-flight (invoice Processing) against the pre-created Pending tx; the
        // later payments.confirmed flips it to Paid.
        let checkout_session_id = CheckoutSessionId::parse_base62(&session_id_str)
            .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

        match services
            .on_hosted_checkout_fulfilled(
                connector.tenant_id,
                checkout_session_id,
                payment_method.id,
                checkout_payment_id,
                None,
            )
            .await
        {
            // Covers the benign duplicates too: a redelivered `fulfilled` whose tx is
            // already materialized (or absent) returns Ok as a logged no-op inside.
            Ok(_) => log::info!(
                "mandate {} attached; checkout session {} materialized in-flight",
                payment_method.id,
                checkout_session_id
            ),
            // The provider has already created the first payment, so any failure
            // here is money committed with nothing materialized. Never ack it:
            // retry via pgmq (the materialization is idempotent) and let it
            // dead-letter visibly if it keeps failing, rather than logging at info
            // and moving on.
            Err(e) => {
                log::error!(
                    "mandate {} attached but checkout session {} could not be materialized; retrying via pgmq: {e:?}",
                    payment_method.id,
                    checkout_session_id
                );
                return Err(e).change_context(errors::AdapterWebhookError::StoreError);
            }
        }
    }

    Ok(())
}

/// PaymentIntent state-change events: succeeded / failed / pending. We resolve
/// the local transaction by the `meteroid_transaction_id` we stamped into charge
/// metadata; when that is absent (GoCardless events carry empty resource
/// metadata) we fall back to the provider charge id persisted as
/// `provider_transaction_id`. If neither resolves the event isn't ours (e.g. a
/// payment created in the provider dashboard) — ack it as a no-op; the
/// reconciliation worker still backstops settlement of every tracked charge.
async fn handle_payment_state_change(
    event: &NormalizedWebhookEvent,
    connector: &Connector,
    store: &Store,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let kind = &event.kind;
    let (meteroid_tx_str, external_tx_id) = match kind {
        NormalizedEventKind::PaymentSucceeded(e) => (
            e.meteroid_transaction_id.as_deref(),
            e.external_transaction_id.as_str(),
        ),
        NormalizedEventKind::PaymentFailed(e) => (
            e.meteroid_transaction_id.as_deref(),
            e.external_transaction_id.as_str(),
        ),
        NormalizedEventKind::PaymentPending(e) => (
            e.meteroid_transaction_id.as_deref(),
            e.external_transaction_id.as_str(),
        ),
        _ => return Ok(()),
    };

    let transaction_id = match meteroid_tx_str {
        Some(s) => PaymentTransactionId::parse_base62(s)
            .change_context(errors::AdapterWebhookError::InvalidMetadata)?,
        None => {
            if external_tx_id.is_empty() {
                return Err(Report::new(errors::AdapterWebhookError::MissingMetadata(
                    "meteroid.transaction_id (and no provider transaction id to correlate by)"
                        .to_string(),
                )));
            }
            match store
                .get_payment_tx_by_provider_transaction_id(connector.tenant_id, external_tx_id)
                .await
                .change_context(errors::AdapterWebhookError::StoreError)?
            {
                Some(tx) => tx.id,
                None => {
                    log::info!(
                        "No local transaction for provider charge {} (event {}); acknowledging no-op",
                        external_tx_id,
                        event.provider_event_id
                    );
                    return Ok(());
                }
            }
        }
    };

    let mut intent = payment_intent_from_event(kind, transaction_id, connector.tenant_id)
        .expect("guarded by outer match");

    // Prefer the provider event's own timestamp for settlement time; the webhook
    // is often processed well after the bank actually settled.
    if intent.status == meteroid_store::domain::enums::PaymentStatusEnum::Settled {
        intent.processed_at = Some(event.occurred_at.naive_utc());
    }

    run_consolidate(store, transaction_id, intent).await?;

    // Processing: the customer finished whatever was asked of them (3DS, a redirect).
    if matches!(
        kind,
        NormalizedEventKind::PaymentPending(pending) if !pending.awaiting_customer
    ) {
        use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
        store
            .clear_awaited_payment_action(connector.tenant_id, transaction_id)
            .await
            .change_context(errors::AdapterWebhookError::StoreError)?;
    }
    Ok(())
}

/// `payment_intent.requires_action`: the charge needs 3DS/SCA. Persist the
/// action on the transaction (it stays Pending) so the portal/dunning can drive
/// the customer through it. Mainly the off-session path, where the synchronous
/// charge couldn't capture it.
async fn handle_payment_requires_action(
    e: &meteroid_store::adapters::payment::events::PaymentRequiresActionEvent,
    connector: &Connector,
    store: &Store,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    use meteroid_store::domain::payment_transactions::{PaymentIntent, PaymentNextAction};

    let meteroid_tx_str = e.meteroid_transaction_id.as_ref().ok_or_else(|| {
        Report::new(errors::AdapterWebhookError::MissingMetadata(
            "meteroid.transaction_id".to_string(),
        ))
    })?;
    let transaction_id = PaymentTransactionId::parse_base62(meteroid_tx_str)
        .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

    let next_action = if let Some(url) = &e.action_url {
        PaymentNextAction::RedirectToUrl { url: url.clone() }
    } else if let Some(secret) = &e.client_secret {
        PaymentNextAction::UseSdk {
            intent_id: e.external_transaction_id.clone(),
            publishable_key: connector.publishable_key().unwrap_or_default().to_string(),
            client_secret: Some(secrecy::SecretString::from(secret.clone())),
        }
    } else {
        log::warn!(
            "requires_action event for tx {} carried no actionable next step",
            transaction_id
        );
        return Ok(());
    };

    let intent = PaymentIntent {
        external_id: e.external_transaction_id.clone(),
        transaction_id,
        tenant_id: connector.tenant_id,
        amount_requested: 0,
        amount_received: None,
        currency: String::new(),
        next_action: Some(next_action),
        status: meteroid_store::domain::enums::PaymentStatusEnum::Pending,
        last_payment_error: None,
        decline_kind: None,
        processed_at: None,
    };

    run_consolidate(store, transaction_id, intent).await
}

/// `payment_method.updated` / `automatically_updated`: refresh the stored card
/// brand/last4/expiry (e.g. Stripe's card-account-updater pushed new details).
async fn handle_payment_method_updated(
    e: &meteroid_store::adapters::payment::events::PaymentMethodUpdatedEvent,
    connector: &Connector,
    store: &Store,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    store
        .update_payment_method_card_details(
            connector.tenant_id,
            &e.external_payment_method_id,
            e.card_brand.clone(),
            e.card_last4.clone(),
            e.card_exp_month,
            e.card_exp_year,
        )
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;
    Ok(())
}

/// `payment_method.detached` / GC mandate cancelled·expired·failed: the method
/// is dead at the provider. Archive our row and clear the customer's current
/// pointer (one tx, idempotent) so renewals stop charging a revoked mandate.
async fn handle_payment_method_detached(
    e: &meteroid_store::adapters::payment::events::PaymentMethodDetachedEvent,
    connector: &Connector,
    store: &Store,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    match store
        .detach_payment_method_by_external_id(connector.tenant_id, &e.external_payment_method_id)
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?
    {
        Some(method_id) => log::info!(
            "Payment method {} detached at provider (external id {}, reason {:?}); archived and unset as current",
            method_id,
            e.external_payment_method_id,
            e.reason,
        ),
        // Not ours (e.g. a mandate created in the provider dashboard) — ack so
        // the webhook isn't retried into the dead-letter queue.
        None => log::warn!(
            "Detached payment method {} has no local row (reason {:?}); acknowledging no-op",
            e.external_payment_method_id,
            e.reason,
        ),
    }
    Ok(())
}

/// Run the resolved intent through the shared settlement pipeline.
async fn run_consolidate(
    store: &Store,
    transaction_id: PaymentTransactionId,
    intent: meteroid_store::domain::payment_transactions::PaymentIntent,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    use meteroid_store::domain::enums::PaymentStatusEnum;

    let mut result = consolidate_once(store, transaction_id, intent.clone()).await;

    // The stamped transaction id may point to a rolled-back row while the provider kept the payment
    // (replayed onto a newer row): settle the row holding the provider payment id.
    if let Err(e) = &result
        && matches!(e.current_context(), StoreError::ValueNotFound(_))
        && !intent.external_id.is_empty()
        && let Ok(Some(tx)) = store
            .get_payment_tx_by_provider_transaction_id(intent.tenant_id, &intent.external_id)
            .await
        && tx.id != transaction_id
    {
        log::info!(
            "Webhook transaction {transaction_id} not found; applying it to {} (holds provider payment {})",
            tx.id,
            intent.external_id
        );
        let mut rebound = intent.clone();
        rebound.transaction_id = tx.id;
        result = consolidate_once(store, tx.id, rebound).await;
    }

    // A declined checkout is recorded post-tx (persist_declined_checkout_charge),
    // so a FAILED/cancelled webhook can legitimately arrive with no local row yet:
    // the webhook won the race with that best-effort insert — ack it as a
    // duplicate. But a SUCCEEDED/pending event with no row is NOT a benign
    // duplicate: it can be an orphaned capture (the provider took money but our
    // transaction rolled back before committing the row). Reconciliation cannot
    // recover it (there is no row to poll), so surface it — return the error so
    // pgmq retries and, failing that, dead-letters it for an operator.
    if let Err(e) = &result
        && matches!(e.current_context(), StoreError::ValueNotFound(_))
    {
        if matches!(
            intent.status,
            PaymentStatusEnum::Failed | PaymentStatusEnum::Cancelled
        ) {
            log::warn!(
                "Webhook references transaction {transaction_id} with no local row \
                 (likely a synchronous checkout decline already handled inline); acking as duplicate"
            );
            return Ok(());
        }
        log::error!(
            "Webhook references transaction {transaction_id} (status {:?}) with no local row — \
             possible orphaned capture (provider charged, local tx rolled back); \
             not acking so it retries and surfaces",
            intent.status
        );
    }

    result.change_context(errors::AdapterWebhookError::StoreError)?;
    Ok(())
}

async fn consolidate_once(
    store: &Store,
    transaction_id: PaymentTransactionId,
    intent: meteroid_store::domain::payment_transactions::PaymentIntent,
) -> Result<(), Report<StoreError>> {
    let store_clone = store.clone();
    store
        .transaction(|conn| {
            let store = store_clone.clone();
            let intent = intent.clone();
            async move {
                let existing = store
                    .get_payment_tx_by_id_for_update(conn, transaction_id, intent.tenant_id)
                    .await?;
                store
                    .consolidate_intent_and_transaction_tx(
                        conn,
                        &meteroid_store::domain::entity_activity::Actor::System,
                        existing,
                        intent,
                    )
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
}

/// The refunds a `charge.refunded`-style event named. Providers that list none still name the
/// single refund that triggered the event, so the recorded row keeps the provider's refund id.
fn observed_refunds_from_event(
    e: &meteroid_store::adapters::payment::events::PaymentRefundedEvent,
) -> Vec<ObservedRefund> {
    if !e.refunds.is_empty() {
        return e
            .refunds
            .iter()
            .map(|r| ObservedRefund {
                external_refund_id: r.external_refund_id.clone(),
                amount_minor: r.amount_minor,
                status: r.status,
            })
            .collect();
    }
    if e.external_refund_id.is_empty() {
        return Vec::new();
    }
    // `amount_refunded_minor` is the parent's cumulative total, not this refund's amount.
    vec![ObservedRefund {
        external_refund_id: e.external_refund_id.clone(),
        amount_minor: None,
        status: ObservedRefundStatus::Succeeded,
    }]
}

/// GoCardless `refunds.*`: the event only names the refund, so resolve it (and the charge it
/// belongs to) at the provider before recording it. A refund still in flight is left for its
/// next event or the reconcile sweep.
async fn handle_refund_observed(
    external_refund_id: &str,
    connector: &Connector,
    connector_impl: &dyn PaymentConnector,
    store: &Store,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let snapshot = connector_impl
        .fetch_refund(external_refund_id, None)
        .await
        .change_context(errors::AdapterWebhookError::ProviderError)?;

    if matches!(snapshot.status, RemoteRefundStatus::Pending) {
        log::debug!(
            "Refund {external_refund_id} is still pending at the provider; nothing to record"
        );
        return Ok(());
    }

    let observed = ObservedRefund {
        external_refund_id: snapshot.external_refund_id,
        amount_minor: Some(snapshot.amount_minor),
        status: if matches!(snapshot.status, RemoteRefundStatus::Succeeded) {
            ObservedRefundStatus::Succeeded
        } else {
            ObservedRefundStatus::Failed
        },
    };

    handle_observed_refunds(
        store,
        connector,
        snapshot.external_transaction_id,
        None,
        snapshot.cumulative_refunded_minor,
        vec![observed],
        occurred_at,
    )
    .await
}

/// Stripe `refund.updated` / `charge.refund.updated`: a refund we issued moved at the provider.
/// Settles or fails the REFUND row it names; a refund we never issued is acked — dashboard
/// refunds are recorded from the cumulative `charge.refunded` event instead.
async fn handle_refund_status_change(
    e: &meteroid_store::adapters::payment::events::RefundStatusChangedEvent,
    connector: &Connector,
    store: &Store,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let tenant_id = connector.tenant_id;

    let Some(refund) = find_local_refund(store, tenant_id, &e.external_refund_id).await? else {
        log::info!(
            "No local refund for provider refund {} (status {:?}); acknowledging no-op",
            e.external_refund_id,
            e.status
        );
        return Ok(());
    };

    let external_refund_id = e.external_refund_id.clone();
    let status = match &e.status {
        // Still in flight at the provider: nothing to settle.
        RemoteRefundStatus::Pending => return Ok(()),
        RemoteRefundStatus::Succeeded => RefundSettlementStatus::Settled {
            processed_at: occurred_at.naive_utc(),
            external_refund_id,
        },
        RemoteRefundStatus::Failed { message, .. } => RefundSettlementStatus::Failed {
            error: message.clone(),
            external_refund_id: Some(external_refund_id),
        },
        RemoteRefundStatus::Canceled => RefundSettlementStatus::Failed {
            error: "Cancelled by the payment provider".to_string(),
            external_refund_id: Some(external_refund_id),
        },
    };

    let store_clone = store.clone();
    store
        .transaction(|conn| {
            let store = store_clone.clone();
            let status = status.clone();
            // Unlocked: `settle_refund_tx` locks the parent first, then re-reads this row.
            let refund = refund.clone();
            async move {
                store
                    .settle_refund_tx(conn, &Actor::System, refund, RefundSettlement { status })
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;
    Ok(())
}

/// Record what a provider reports about a payment's refunds. Voluntary movement: REFUND rows are
/// recorded against the parent payment and the invoice is never touched. Resolves and locks the
/// parent the way [`handle_reversal`] does, then lets the store settle the refund rows it already
/// knows before recording the remaining delta. A charge we don't know is acked as a no-op.
#[allow(clippy::too_many_arguments)]
async fn handle_observed_refunds(
    store: &Store,
    connector: &Connector,
    external_transaction_id: String,
    meteroid_transaction_id: Option<String>,
    cumulative_refunded_minor: Option<i64>,
    refunds: Vec<ObservedRefund>,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let tenant_id = connector.tenant_id;

    let transaction_id = match resolve_transaction_id(
        store,
        tenant_id,
        meteroid_transaction_id.as_deref(),
        &external_transaction_id,
    )
    .await?
    {
        Some(id) => id,
        None => {
            log::warn!(
                "No local transaction for refunds observed on charge {}; acknowledging no-op",
                external_transaction_id
            );
            return Ok(());
        }
    };

    // Providers that report no cumulative total (GoCardless) leave it to be derived from what
    // the parent already carries plus the observed refunds we have not settled yet.
    let unsettled_minor = match cumulative_refunded_minor {
        Some(_) => 0,
        None => sum_unsettled_refunds(store, tenant_id, &refunds).await?,
    };

    let observed_at = occurred_at.naive_utc();
    let store_clone = store.clone();
    store
        .transaction(|conn| {
            let store = store_clone.clone();
            let external_transaction_id = external_transaction_id.clone();
            let refunds = refunds.clone();
            async move {
                let parent = store
                    .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                    .await?;
                let observed = ObservedRefunds {
                    external_transaction_id,
                    cumulative_refunded_minor: cumulative_refunded_minor
                        .unwrap_or(parent.amount_refunded + unsettled_minor),
                    refund_ids: refunds,
                    observed_at,
                };
                store
                    .record_observed_refunds_tx(conn, &Actor::System, parent, observed)
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;
    Ok(())
}

/// Sum of the observed refunds that succeeded and that no settled REFUND row accounts for yet —
/// the raise a provider without a cumulative figure implies on the parent's refunded total.
async fn sum_unsettled_refunds(
    store: &Store,
    tenant_id: TenantId,
    refunds: &[ObservedRefund],
) -> Result<i64, Report<errors::AdapterWebhookError>> {
    let mut sum = 0;
    for refund in refunds
        .iter()
        .filter(|r| r.status == ObservedRefundStatus::Succeeded)
    {
        let settled = find_local_refund(store, tenant_id, &refund.external_refund_id)
            .await?
            .is_some_and(|r| matches!(r.status, PaymentStatusEnum::Settled));
        if !settled {
            sum += refund.amount_minor.unwrap_or_default();
        }
    }
    Ok(sum)
}

/// The REFUND row we carry for a provider refund id — one we issued, or one recorded from an
/// earlier delivery.
async fn find_local_refund(
    store: &Store,
    tenant_id: TenantId,
    external_refund_id: &str,
) -> Result<Option<PaymentTransaction>, Report<errors::AdapterWebhookError>> {
    Ok(store
        .get_payment_tx_by_provider_transaction_id(tenant_id, external_refund_id)
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?
        .filter(|t| matches!(t.payment_type, PaymentTypeEnum::Refund)))
}

/// Apply an INVOLUNTARY claw-back (chargeback, bank recall, dispute funds withdrawn, late
/// failure). Resolves the local transaction (by our stamped id when present and parseable, else
/// the provider charge id), locks it, and runs the reversal: a dispute keeps the invoice paid, a
/// failed collection reopens it. Idempotent under redelivery; a charge we don't know is acked as
/// a no-op.
#[allow(clippy::too_many_arguments)]
async fn handle_reversal(
    store: &Store,
    connector: &Connector,
    external_transaction_id: String,
    meteroid_transaction_id: Option<String>,
    amount: ReversalAmount,
    kind: ReversalKind,
    disputed: bool,
    reason: Option<String>,
    provider_ref: Option<String>,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let tenant_id = connector.tenant_id;

    let transaction_id = match resolve_transaction_id(
        store,
        tenant_id,
        meteroid_transaction_id.as_deref(),
        &external_transaction_id,
    )
    .await?
    {
        Some(id) => id,
        None => {
            log::warn!(
                "No local transaction for {:?} reversal of provider charge {}; acknowledging no-op",
                kind,
                external_transaction_id
            );
            return Ok(());
        }
    };

    let reversal = TransactionReversal {
        external_transaction_id,
        amount,
        kind,
        reason,
        reversed_at: occurred_at.naive_utc(),
        disputed,
        provider_ref,
    };

    let store_clone = store.clone();
    store
        .transaction(|conn| {
            let store = store_clone.clone();
            let reversal = reversal.clone();
            async move {
                let existing = store
                    .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                    .await?;
                store
                    .reverse_transaction_tx(conn, &Actor::System, existing, reversal)
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;
    Ok(())
}

/// Apply a dispute/chargeback resolution that returned previously clawed-back
/// funds (Stripe `charge.dispute.funds_reinstated`, GoCardless
/// `chargeback_cancelled`). Inverse of [`handle_reversal`]: restores the
/// transaction's settled amount (re-closing an invoice a failed collection reopened).
/// Idempotent under redelivery; a charge we don't know is acked as a no-op.
#[allow(clippy::too_many_arguments)]
async fn handle_reinstatement(
    store: &Store,
    connector: &Connector,
    external_transaction_id: String,
    meteroid_transaction_id: Option<String>,
    reinstated_amount_minor: Option<i64>,
    reason: String,
    provider_ref: Option<String>,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let tenant_id = connector.tenant_id;

    let transaction_id = match resolve_transaction_id(
        store,
        tenant_id,
        meteroid_transaction_id.as_deref(),
        &external_transaction_id,
    )
    .await?
    {
        Some(id) => id,
        None => {
            log::warn!(
                "No local transaction for reinstatement of provider charge {} (reason {}); acknowledging no-op",
                external_transaction_id,
                reason
            );
            return Ok(());
        }
    };

    let reinstatement = TransactionReinstatement {
        external_transaction_id,
        reinstated_amount_minor,
        reason,
        reinstated_at: occurred_at.naive_utc(),
        provider_ref,
    };

    let store_clone = store.clone();
    store
        .transaction(|conn| {
            let store = store_clone.clone();
            let reinstatement = reinstatement.clone();
            async move {
                let existing = store
                    .get_payment_tx_by_id_for_update(conn, transaction_id, tenant_id)
                    .await?;
                store
                    .reinstate_transaction_tx(conn, &Actor::System, existing, reinstatement)
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;
    Ok(())
}

/// Resolve a local transaction id from a webhook: prefer our stamped
/// `meteroid.transaction_id` when present and parseable, else fall back to the
/// provider charge id (GoCardless events carry no usable metadata; Stripe
/// refund/dispute events carry only the parent charge/intent id).
async fn resolve_transaction_id(
    store: &Store,
    tenant_id: TenantId,
    meteroid_transaction_id: Option<&str>,
    external_transaction_id: &str,
) -> Result<Option<PaymentTransactionId>, Report<errors::AdapterWebhookError>> {
    if let Some(s) = meteroid_transaction_id
        && let Ok(id) = PaymentTransactionId::parse_base62(s)
    {
        return Ok(Some(id));
    }
    Ok(store
        .get_payment_tx_by_provider_transaction_id(tenant_id, external_transaction_id)
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?
        .map(|t| t.id))
}
