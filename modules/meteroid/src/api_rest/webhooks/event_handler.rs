//! Provider-agnostic dispatcher for [`NormalizedWebhookEvent`].
//!
//! The webhook router verifies the signature and parses the payload via the
//! per-provider adapter ([`PaymentConnector::parse_event`]), then hands the
//! normalized event to this dispatcher. The dispatcher only sees the closed
//! enum [`NormalizedEventKind`] — no Stripe/GoCardless/Adyen types leak in.

use crate::errors;
use common_domain::ids::{
    BaseId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, PaymentTransactionId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::{Report, ResultExt};
use meteroid_store::Store;
use meteroid_store::adapters::payment::PaymentConnector;
use meteroid_store::adapters::payment::bridge::payment_intent_from_event;
use meteroid_store::adapters::payment::events::{NormalizedEventKind, NormalizedWebhookEvent};
use meteroid_store::domain::connectors::Connector;
use meteroid_store::domain::{CustomerPatch, CustomerPaymentMethodNew};
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

/// Dispatch a verified, parsed webhook event onto the store. Runs in a spawned
/// task after the router has already responded 200 OK to the provider, so it
/// must not panic — log and return on error.
pub async fn handle_normalized_event(
    event: NormalizedWebhookEvent,
    connector: &Connector,
    connector_impl: &dyn PaymentConnector,
    store: Store,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    log::info!(
        "Processing webhook event {} (type={:?})",
        event.provider_event_id,
        event.provider_event_type,
    );

    match &event.kind {
        NormalizedEventKind::PaymentMethodAttached(e) => {
            handle_payment_method_attached(e, connector, connector_impl, &store).await
        }
        NormalizedEventKind::PaymentSucceeded(_)
        | NormalizedEventKind::PaymentFailed(_)
        | NormalizedEventKind::PaymentPending(_) => {
            handle_payment_state_change(&event.kind, connector, &store).await
        }
        NormalizedEventKind::PaymentRequiresAction(e) => {
            handle_payment_requires_action(e, connector, &store).await
        }
        NormalizedEventKind::PaymentMethodUpdated(e) => {
            handle_payment_method_updated(e, connector, &store).await
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
        NormalizedEventKind::Acknowledged { reason } => {
            log::debug!(
                "Acknowledged unhandled event {}: {}",
                event.provider_event_id,
                reason
            );
            Ok(())
        }
        // Not handled yet (requires_action, refunded, disputes, method
        // updated/expiring): log and ack so the provider stops retrying.
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

/// SetupIntent-equivalent: customer just attached a new payment method via
/// hosted UI. We fetch the canonical snapshot from the provider, upsert it,
/// and mark it as the customer's current payment method.
async fn handle_payment_method_attached(
    e: &meteroid_store::adapters::payment::events::PaymentMethodAttachedEvent,
    connector: &Connector,
    connector_impl: &dyn PaymentConnector,
    store: &Store,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    // Fetch the canonical snapshot. For GoCardless this also recovers our
    // `meteroid.*` ids, since mandate webhook events carry empty metadata.
    let snapshot = connector_impl
        .fetch_payment_method(
            connector,
            &e.external_payment_method_id,
            &e.external_customer_id,
        )
        .await
        .change_context(errors::AdapterWebhookError::ProviderError)?;

    // Event metadata (Stripe) if present, else the fetched resource (GoCardless).
    let connection_id_str = e
        .meteroid_connection_id
        .as_deref()
        .or(snapshot.meteroid_connection_id.as_deref())
        .ok_or_else(|| {
            Report::new(errors::AdapterWebhookError::MissingMetadata(
                "meteroid.connection_id".to_string(),
            ))
        })?;
    let connection_id = CustomerConnectionId::parse_base62(connection_id_str)
        .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

    let customer_id_str = e
        .meteroid_customer_id
        .as_deref()
        .or(snapshot.meteroid_customer_id.as_deref())
        .ok_or_else(|| {
            Report::new(errors::AdapterWebhookError::MissingMetadata(
                "meteroid.customer_id".to_string(),
            ))
        })?;
    let customer_id = CustomerId::parse_base62(customer_id_str)
        .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

    // Cross-tenant / hijack defense: verify the connection belongs to this
    // connector's tenant and is owned by the named customer.
    use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
    let connection = store
        .get_connection_by_id(&connector.tenant_id, &connection_id)
        .await
        .change_context(errors::AdapterWebhookError::InvalidMetadata)
        .attach("webhook references a connection that does not belong to its connector's tenant")?;
    if connection.customer_id != customer_id {
        return Err(Report::new(errors::AdapterWebhookError::InvalidMetadata)
            .attach("webhook connection_id / customer_id pair is inconsistent"));
    }

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
        is_tax_exempt: None,
        custom_taxes: None,
        connected_account_id: None,
    };
    store
        .patch_customer(customer_id.as_uuid(), connector.tenant_id, patch)
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;

    Ok(())
}

/// PaymentIntent state-change events: succeeded / failed / pending. We look
/// up the local transaction by the `meteroid_transaction_id` we stamped into
/// metadata when creating the charge, then run the existing consolidation
/// pipeline.
async fn handle_payment_state_change(
    kind: &NormalizedEventKind,
    connector: &Connector,
    store: &Store,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    let meteroid_tx_str = match kind {
        NormalizedEventKind::PaymentSucceeded(e) => e.meteroid_transaction_id.as_ref(),
        NormalizedEventKind::PaymentFailed(e) => e.meteroid_transaction_id.as_ref(),
        NormalizedEventKind::PaymentPending(e) => e.meteroid_transaction_id.as_ref(),
        _ => return Ok(()),
    };

    let meteroid_tx_str = meteroid_tx_str.ok_or_else(|| {
        Report::new(errors::AdapterWebhookError::MissingMetadata(
            "meteroid.transaction_id".to_string(),
        ))
    })?;

    let transaction_id = PaymentTransactionId::parse_base62(meteroid_tx_str)
        .change_context(errors::AdapterWebhookError::InvalidMetadata)?;

    let intent = payment_intent_from_event(kind, transaction_id, connector.tenant_id)
        .expect("guarded by outer match");

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
            client_secret: Some(secret.clone()),
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

/// Run the resolved intent through the shared settlement pipeline.
async fn run_consolidate(
    store: &Store,
    transaction_id: PaymentTransactionId,
    intent: meteroid_store::domain::payment_transactions::PaymentIntent,
) -> Result<(), Report<errors::AdapterWebhookError>> {
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
                    .consolidate_intent_and_transaction_tx(conn, existing, intent)
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .change_context(errors::AdapterWebhookError::StoreError)?;
    Ok(())
}

fn stripe_publishable_key(connector: &Connector) -> Option<String> {
    match &connector.data {
        Some(meteroid_store::domain::connectors::ProviderData::Stripe(d)) => {
            Some(d.api_publishable_key.clone())
        }
        _ => None,
    }
}
