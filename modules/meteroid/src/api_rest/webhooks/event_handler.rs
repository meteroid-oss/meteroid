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
use meteroid_store::adapters::payment::PaymentConnector;
use meteroid_store::adapters::payment::bridge::payment_intent_from_event;
use meteroid_store::adapters::payment::events::{NormalizedEventKind, NormalizedWebhookEvent};
use meteroid_store::domain::connectors::Connector;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::{CustomerPatch, CustomerPaymentMethodNew};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::payment_transactions::{
    PaymentTransactionInterface, ReversalAmount, TransactionReinstatement, TransactionReversal,
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
        // ── money reversals ───────────────────────────────────────────────
        // Stripe `charge.refunded` (cumulative, partial-aware).
        NormalizedEventKind::PaymentRefunded(e) => {
            handle_reversal(
                &store,
                connector,
                e.external_transaction_id.clone(),
                None,
                ReversalAmount::Cumulative(e.amount_refunded_minor),
                "charge.refunded".to_string(),
                event.occurred_at,
            )
            .await
        }
        // Provider-side (dashboard) refund whose event carries no amounts
        // (GoCardless): resolve the parent payment and its cumulative refunded
        // total from the provider, then run the normal reversal path.
        NormalizedEventKind::RefundObserved { external_refund_id } => {
            let refund = connector_impl
                .fetch_refund(connector, external_refund_id)
                .await
                .change_context(errors::AdapterWebhookError::ProviderError)?;
            handle_reversal(
                &store,
                connector,
                refund.external_transaction_id.clone(),
                None,
                ReversalAmount::Cumulative(refund.cumulative_refunded_minor),
                format!("refund.{external_refund_id}"),
                event.occurred_at,
            )
            .await
        }
        // GoCardless `charged_back` / `late_failure_settled` (full amount).
        NormalizedEventKind::PaymentReversed(e) => {
            handle_reversal(
                &store,
                connector,
                e.external_transaction_id.clone(),
                e.meteroid_transaction_id.clone(),
                ReversalAmount::Full,
                e.reason.clone(),
                event.occurred_at,
            )
            .await
        }
        // GoCardless `chargeback_cancelled`: the bank returned the full
        // clawed-back amount — restore the settlement, re-close the invoice.
        NormalizedEventKind::PaymentReinstated(e) => {
            handle_reinstatement(
                &store,
                connector,
                e.external_transaction_id.clone(),
                e.meteroid_transaction_id.clone(),
                None,
                e.reason.clone(),
                event.occurred_at,
            )
            .await
        }
        // Stripe dispute: funds actually withdrawn. Disputes can be partial
        // (`dispute.amount` <= charge amount), so claw back only that delta on
        // top of whatever was already refunded.
        NormalizedEventKind::DisputeFundsWithdrawn(e) => {
            handle_reversal(
                &store,
                connector,
                e.external_transaction_id.clone(),
                None,
                ReversalAmount::Incremental(e.amount_minor),
                format!(
                    "dispute.{}",
                    e.reason.as_deref().unwrap_or("funds_withdrawn")
                ),
                event.occurred_at,
            )
            .await
        }
        // Stripe dispute resolved in the merchant's favor: the withdrawn
        // dispute amount came back — undo the reversal, re-close the invoice.
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
        .fetch_payment_method(
            connector,
            &e.external_payment_method_id,
            &e.external_customer_id,
        )
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

    attach_payment_method_from_snapshot(snapshot, connector, store, services).await
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
    let snapshot = connector_impl
        .complete_mandate_setup(connector, provider_intent_id)
        .await
        .change_context(errors::AdapterWebhookError::ProviderError)?;

    attach_payment_method_from_snapshot(snapshot, connector, store, services).await
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
    // A combined mandate+payment CHECKOUT Billing Request: the provider already
    // created the first payment, so instead of charging an invoice we materialize
    // the subscription in-flight against the pre-created checkout transaction.
    let checkout_session_to_complete = snapshot.meteroid_checkout_session_id.clone();
    let checkout_payment_id = snapshot.payment_request_payment.clone();
    // Stable across pgmq retries of this same webhook (mandate id + invoice are
    // both fixed for a given fulfillment) and unique to this logical charge — the
    // provider-idempotency seed that makes the charge safe to retry.
    let mandate_ref = snapshot.external_payment_method_id.clone();

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
        })
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;

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
        tax_status: None,
        exemption_reason: None,
        custom_taxes: None,
        connected_account_id: None,
    };
    store
        .patch_customer(
            meteroid_store::domain::entity_activity::Actor::System,
            connector.tenant_id,
            patch,
        )
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;

    if let Some(invoice_id_str) = invoice_to_charge {
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
                    StoreError::PaymentError(_) | StoreError::BillingError
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

    run_consolidate(store, transaction_id, intent).await
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
            publishable_key: stripe_publishable_key(connector).unwrap_or_default(),
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

    let store_clone = store.clone();
    let result = store
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
        .await;

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

/// Apply a post-settlement reversal (refund / chargeback / dispute funds
/// withdrawn). Resolves the local transaction (by our stamped id when present
/// and parseable, else the provider charge id), locks it, and runs the reversal
/// which reopens the invoice. Idempotent under redelivery; a charge we don't
/// know is acked as a no-op.
#[allow(clippy::too_many_arguments)]
async fn handle_reversal(
    store: &Store,
    connector: &Connector,
    external_transaction_id: String,
    meteroid_transaction_id: Option<String>,
    amount: ReversalAmount,
    reason: String,
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
                "No local transaction for reversal of provider charge {} (reason {}); acknowledging no-op",
                external_transaction_id,
                reason
            );
            return Ok(());
        }
    };

    let reversal = TransactionReversal {
        external_transaction_id,
        amount,
        reason,
        reversed_at: occurred_at.naive_utc(),
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
/// transaction's settled amount and re-closes the invoice. Idempotent under
/// redelivery; a charge we don't know is acked as a no-op.
async fn handle_reinstatement(
    store: &Store,
    connector: &Connector,
    external_transaction_id: String,
    meteroid_transaction_id: Option<String>,
    reinstated_amount_minor: Option<i64>,
    reason: String,
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

fn stripe_publishable_key(connector: &Connector) -> Option<String> {
    match &connector.data {
        Some(meteroid_store::domain::connectors::ProviderData::Stripe(d)) => {
            Some(d.api_publishable_key.clone())
        }
        _ => None,
    }
}
