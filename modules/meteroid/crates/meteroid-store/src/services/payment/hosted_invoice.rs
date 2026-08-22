//! In-flow hosted INVOICE payment initiation for webhook-less
//! (`PollingRequired`) providers. Mirrors `initiate_hosted_checkout`: a
//! committed Pending transaction is pre-created for `amount_due` and the
//! intent minted with `capture: true`, metadata naming the invoice AND the
//! transaction — the hosted capture is the single charge. Single-intent
//! discipline: re-initiation first cancels the stored prior intent; an
//! uncancelable one is ADOPTED through completion instead of replaced, so at
//! most ONE capturable intent exists per invoice at any time.

use crate::StoreResult;
use crate::domain::SetupIntent;
use crate::domain::payment_transactions::PaymentNextAction;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::payment::hosted_setup::HostedSetupOutcome;
use crate::services::payment::method::CancelPendingIntentOutcome;
use common_domain::ids::{BaseId, CustomerConnectionId, InvoiceId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::enums::PaymentStatusEnum;
use diesel_models::invoices::InvoiceRow;
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// One initiation pass: either the hosted intent to redirect to, or an
/// instruction to ADOPT the invoice's uncancelable prior intent.
enum HostedInvoiceInitiation {
    Intent(Box<SetupIntent>),
    AdoptPriorIntent {
        connection_id: CustomerConnectionId,
        intent_id: String,
    },
}

impl Services {
    /// Start (or resume) an in-flow hosted payment for `invoice_id`; the
    /// returned [`SetupIntent`]'s `client_secret` carries the hosted page URL.
    /// While a hosted attempt is still Pending, the SAME stored
    /// intent/redirect is returned instead of minting a second capturable
    /// intent. The invoice row is locked FOR UPDATE, serializing concurrent
    /// initiations and the sweeper's close-out.
    pub(in crate::services) async fn initiate_hosted_invoice_payment(
        &self,
        tenant_id: TenantId,
        connection_id: CustomerConnectionId,
        invoice_id: InvoiceId,
        return_url: Option<String>,
    ) -> StoreResult<SetupIntent> {
        // At most twice: once more after an adoption resolves the prior
        // attempt as declined/dead.
        for adoption_attempt in 0..2u8 {
            let return_url = return_url.clone();

            let outcome = self
                .store
                .transaction(|conn| {
                    async move {
                        let invoice =
                            InvoiceRow::select_for_update_by_id(conn, tenant_id, invoice_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;

                        // Same payability gates as `process_invoice_payment_tx`.
                        if invoice.invoice.consolidated_into_invoice_id.is_some() {
                            return Err(Report::new(StoreError::BillingError)
                                .attach("Cannot pay an invoice merged into a consolidated parent"));
                        }
                        if invoice.invoice.status != diesel_models::enums::InvoiceStatusEnum::Draft
                            && invoice.invoice.status
                                != diesel_models::enums::InvoiceStatusEnum::Finalized
                        {
                            return Err(Report::new(StoreError::BillingError)
                                .attach("Cannot process payment for this invoice status"));
                        }
                        if invoice.invoice.amount_due <= 0 {
                            return Err(Report::new(StoreError::BillingError)
                                .attach("Invoice has no amount due"));
                        }

                        // Caller-supplied connection: must belong to the
                        // invoice's customer (customer boundary).
                        let connection = CustomerConnectionDetailsRow::get_by_id(
                            conn,
                            &tenant_id,
                            &connection_id,
                        )
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?;
                        if connection.customer.id != invoice.invoice.customer_id {
                            return Err(Report::new(StoreError::InvalidArgument(
                                "Connection does not belong to the invoice's customer".to_string(),
                            )));
                        }
                        let provider: crate::domain::enums::ConnectorProviderEnum =
                            connection.connector.provider.clone().into();
                        let connector_id = connection.connector.id;

                        let existing = PaymentTransactionRow::list_by_invoice_id(
                            conn, invoice_id, tenant_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                        // One attempt at a time: an in-flight hosted attempt is
                        // RETURNED; any other in-flight payment refuses a second.
                        if let Some(pending) = existing
                            .iter()
                            .find(|tx| tx.transaction.status == PaymentStatusEnum::Pending)
                        {
                            let row = &pending.transaction;
                            if let (Some(intent_id), Some(url)) = (
                                row.pending_provider_intent_id.clone(),
                                row.next_action.clone().and_then(|v| {
                                    match serde_json::from_value::<PaymentNextAction>(v) {
                                        Ok(PaymentNextAction::RedirectToUrl { url }) => Some(url),
                                        _ => None,
                                    }
                                }),
                            ) {
                                let stored_connection =
                                    row.pending_connection_id.unwrap_or(connection_id);
                                return Ok(HostedInvoiceInitiation::Intent(Box::new(SetupIntent {
                                    intent_id,
                                    client_secret: url,
                                    public_key: secrecy::SecretString::from(String::new()),
                                    provider,
                                    connector_id,
                                    connection_id: stored_connection,
                                })));
                            }
                            return Err(Report::new(StoreError::PaymentError(
                                "A payment for this invoice is already being processed. Please \
                                 wait for it to complete before attempting another payment."
                                    .to_string(),
                            )));
                        }

                        // Over-payment guards (no Pending rows remain here).
                        let active_payment_sum: i64 = existing
                            .iter()
                            .filter(|tx| {
                                matches!(
                                    tx.transaction.status,
                                    PaymentStatusEnum::Ready | PaymentStatusEnum::Settled
                                )
                            })
                            .map(|tx| tx.transaction.amount - tx.transaction.amount_refunded)
                            .sum();
                        if active_payment_sum >= invoice.invoice.total {
                            return Err(Report::new(StoreError::PaymentError(format!(
                                "Invoice already has sufficient payments. Total: {}, Already paid: {}",
                                invoice.invoice.total, active_payment_sum
                            ))));
                        }
                        if active_payment_sum + invoice.invoice.amount_due > invoice.invoice.total {
                            return Err(Report::new(StoreError::PaymentError(format!(
                                "Payment of {} would exceed invoice total. Already paid: {}, Total: {}",
                                invoice.invoice.amount_due, active_payment_sum, invoice.invoice.total
                            ))));
                        }

                        // Single-intent discipline: cancel the prior stored
                        // intent BEFORE minting a replacement; adopt it when
                        // the provider refuses (payment underway/captured).
                        if let Some(prior) = PaymentTransactionRow::latest_with_pending_intent_by_invoice_id(
                            conn, invoice_id, tenant_id,
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                            && let Some(prior_intent) = prior.pending_provider_intent_id.clone()
                        {
                            let prior_connection =
                                prior.pending_connection_id.unwrap_or(connection_id);
                            match self
                                .cancel_pending_hosted_intent(
                                    conn,
                                    &tenant_id,
                                    &prior_connection,
                                    &prior_intent,
                                )
                                .await?
                            {
                                CancelPendingIntentOutcome::Cancelled => {
                                    // Dead at the provider: stop sweeping it.
                                    PaymentTransactionRow::clear_pending_intent_if_matches(
                                        conn,
                                        tenant_id,
                                        prior.id,
                                        &prior_intent,
                                    )
                                    .await
                                    .map_err(Into::<Report<StoreError>>::into)?;
                                    log::info!(
                                        "cancelled superseded hosted invoice intent {prior_intent} \
                                         for invoice {invoice_id}"
                                    );
                                }
                                CancelPendingIntentOutcome::NotCancelable => {
                                    return Ok(HostedInvoiceInitiation::AdoptPriorIntent {
                                        connection_id: prior_connection,
                                        intent_id: prior_intent,
                                    });
                                }
                            }
                        }

                        let transaction_id = common_domain::ids::PaymentTransactionId::new();

                        let invoice_ctx =
                            crate::adapters::payment::model::HostedInvoicePaymentContext {
                                invoice_id: invoice_id.as_base62(),
                                transaction_id: transaction_id.as_base62(),
                                amount_minor: invoice.invoice.amount_due,
                                currency: invoice.invoice.currency.clone(),
                            };

                        let setup_intent = self
                            .create_setup_intent_internal(
                                conn,
                                &tenant_id,
                                &connection_id,
                                None,
                                Some(invoice_id),
                                None,
                                Some(invoice_ctx),
                                return_url,
                            )
                            .await?;

                        let next_action = PaymentNextAction::RedirectToUrl {
                            url: setup_intent.client_secret.clone(),
                        };

                        // Pre-create the committed Pending transaction the
                        // hosted capture is recorded onto, carrying the intent
                        // id so the sweeper can recover a lost-return capture.
                        let row = PaymentTransactionRowNew {
                            id: transaction_id,
                            tenant_id,
                            invoice_id: Some(invoice_id),
                            provider_transaction_id: None,
                            amount: invoice.invoice.amount_due,
                            currency: invoice.invoice.currency.clone(),
                            payment_method_id: None,
                            status: PaymentStatusEnum::Pending,
                            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
                            error_type: None,
                            processed_at: None,
                            checkout_session_id: None,
                            pending_plan_version_id: None,
                            next_action: serde_json::to_value(&next_action).ok(),
                            // The customer initiates this from the portal.
                            initiated_by_customer_id: Some(invoice.invoice.customer_id),
                            pending_provider_intent_id: Some(setup_intent.intent_id.clone()),
                            pending_connection_id: Some(setup_intent.connection_id),
                        };
                        row.insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;

                        Ok(HostedInvoiceInitiation::Intent(Box::new(setup_intent)))
                    }
                    .scope_boxed()
                })
                .await?;

            let (prior_connection, prior_intent) = match outcome {
                HostedInvoiceInitiation::Intent(intent) => return Ok(*intent),
                HostedInvoiceInitiation::AdoptPriorIntent {
                    connection_id,
                    intent_id,
                } => (connection_id, intent_id),
            };

            // Adoption: run the uncancelable prior intent through the SAME
            // completion routine (records, never charges) instead of minting
            // a second capturable intent.
            log::warn!(
                "hosted invoice payment for invoice {invoice_id}: prior intent {prior_intent} is \
                 not cancelable; adopting it through completion instead of re-minting"
            );
            let setup_outcome = self
                .complete_hosted_setup_with_attempts(prior_connection, prior_intent.clone(), 1)
                .await?;

            match setup_outcome {
                HostedSetupOutcome::InvoiceCharged(_) => {
                    // Never show a new hosted page over recovered money.
                    return Err(Report::new(StoreError::InvalidArgument(
                        "A previous payment for this invoice was recovered and is being \
                         finalized; please refresh the page."
                            .to_string(),
                    )));
                }
                HostedSetupOutcome::HeldForReview { .. } => {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "A previous payment on this invoice requires manual review; please \
                         contact support before retrying."
                            .to_string(),
                    )));
                }
                HostedSetupOutcome::PaymentFailed { .. } | HostedSetupOutcome::SetupFailed
                    if adoption_attempt == 0 =>
                {
                    // Resolved as declined/dead: now cancelable — loop once to
                    // mint fresh.
                    continue;
                }
                HostedSetupOutcome::Processing => {
                    return Err(Report::new(StoreError::InvalidArgument(
                        "A payment for this invoice is still being processed; please retry \
                         shortly."
                            .to_string(),
                    )));
                }
                other => {
                    log::error!(
                        "hosted invoice payment for invoice {invoice_id}: adopted intent \
                         {prior_intent} resolved as {other:?}; refusing to mint a replacement"
                    );
                    return Err(Report::new(StoreError::InvalidArgument(
                        "Unable to start a new hosted payment for this invoice; please retry \
                         later or contact support."
                            .to_string(),
                    )));
                }
            }
        }

        // Unreachable: the second iteration always returns or errors above.
        Err(Report::new(StoreError::InvalidArgument(
            "Unable to start a hosted payment for this invoice; please retry later.".to_string(),
        )))
    }
}
