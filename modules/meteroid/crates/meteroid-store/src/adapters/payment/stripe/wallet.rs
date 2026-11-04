//! Wallet top-ups on Stripe: the PaymentIntent is created unconfirmed and its id persisted before
//! it is confirmed, so no client secret exists for an intent the ledger does not know.
use super::{
    StripeConnector, decline_kind_from_error, flatten_payment_error, intent_to_outcome,
    map_stripe_error, stripe_error_code, to_stripe_setup_pm_type,
};
use crate::adapters::payment::connector::WalletOps;
use crate::adapters::payment::error::ConnectorError;
use crate::adapters::payment::model::{
    ChargeFailure, ChargeOutcome, DeclineKind, PrepareWalletChargeRequest,
};
use crate::domain::PaymentStatusEnum;
use crate::domain::payment_transactions::PaymentTransaction;
use async_trait::async_trait;
use common_domain::ids::BaseId;
use error_stack::Report;
use std::collections::HashMap;
use stripe_client::error::StripeError;
use stripe_client::payment_intents::{
    CancelPaymentIntentRequest, ConfirmPaymentIntentRequest, PaymentIntentApi,
    PaymentIntentRequest, StripePaymentIntent, StripePaymentStatus,
};
use uuid::Uuid;

/// Stamped on every prepared intent: webhook handling keys off it.
pub(in crate::adapters::payment) const WALLET_PAYMENT_FLOW: &str = "wallet_prepared_v1";
pub(in crate::adapters::payment) const PAYMENT_FLOW_METADATA_KEY: &str = "meteroid.payment_flow";

fn mismatch(message: &str) -> Report<ConnectorError> {
    Report::new(ConnectorError::Charge(message.to_owned()))
}

impl StripeConnector {
    fn validate_prepared(
        &self,
        intent: &StripePaymentIntent,
        transaction: &PaymentTransaction,
    ) -> Result<(), Report<ConnectorError>> {
        if transaction.provider_transaction_id.as_deref() != Some(intent.id.as_str())
            || intent.amount != transaction.amount
            || !intent.currency.eq_ignore_ascii_case(&transaction.currency)
            || intent.metadata.get("meteroid.tenant_id") != Some(&transaction.tenant_id.as_base62())
            || intent.metadata.get("meteroid.transaction_id") != Some(&transaction.id.as_base62())
        {
            return Err(mismatch(
                "the provider payment differs from the recorded top-up payment",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl WalletOps for StripeConnector {
    async fn prepare_wallet_charge(
        &self,
        request: PrepareWalletChargeRequest<'_>,
    ) -> Result<String, Report<ConnectorError>> {
        let prepared = Self::client()
            .create_payment_intent(
                PaymentIntentRequest {
                    amount: request.amount_minor,
                    currency: request.currency.to_ascii_lowercase(),
                    customer: Some(request.customer_external_id.to_owned()),
                    payment_method: request.payment_method_external_id.to_owned(),
                    metadata: HashMap::from([
                        (
                            "meteroid.tenant_id".to_owned(),
                            self.connector.tenant_id.as_base62(),
                        ),
                        (
                            "meteroid.transaction_id".to_owned(),
                            request.transaction_id.as_base62(),
                        ),
                        (
                            PAYMENT_FLOW_METADATA_KEY.to_owned(),
                            WALLET_PAYMENT_FLOW.to_owned(),
                        ),
                    ]),
                    confirm: false,
                    off_session: None,
                    return_url: None,
                    setup_mandate_details: None,
                    capture_method: Default::default(),
                    payment_method_types: to_stripe_setup_pm_type(&request.payment_method_type)
                        .into_iter()
                        .collect(),
                },
                &self.secret_key(),
                // Nothing is charged here, so a fresh key per attempt costs nothing and keeps a
                // cached 5xx from blocking the top-up for a day.
                format!("{}:{}", request.idempotency_key.as_str(), Uuid::new_v4()),
            )
            .await
            .map_err(|failure| match failure {
                StripeError::Stripe(ref response)
                    if matches!(response.http_status, 400 | 402 | 404)
                        && matches!(
                            response.error_type.as_str(),
                            "invalid_request_error" | "card_error"
                        ) =>
                {
                    Report::new(ConnectorError::WalletPreparationRejected(
                        failure.to_string(),
                    ))
                }
                other => map_stripe_error(other),
            })?;
        if prepared.metadata.get("meteroid.transaction_id")
            != Some(&request.transaction_id.as_base62())
            || prepared.metadata.get("meteroid.tenant_id")
                != Some(&self.connector.tenant_id.as_base62())
            || prepared.amount != request.amount_minor
            || !prepared.currency.eq_ignore_ascii_case(request.currency)
        {
            return Err(mismatch(
                "the prepared provider payment does not match its request",
            ));
        }
        Ok(prepared.id)
    }

    async fn resume_wallet_charge(
        &self,
        transaction: &PaymentTransaction,
        on_session: bool,
    ) -> Result<ChargeOutcome, Report<ConnectorError>> {
        if self.connector.tenant_id != transaction.tenant_id {
            return Err(mismatch("the connector belongs to another tenant"));
        }
        let id = transaction
            .provider_transaction_id
            .as_deref()
            .ok_or_else(|| mismatch("a top-up payment is recorded before it is confirmed"))?;
        let secret = self.secret_key();
        let client = Self::client();
        let mut current = client
            .get_payment_intent(id, &secret)
            .await
            .map_err(map_stripe_error)?;
        self.validate_prepared(&current, transaction)?;
        let cancel_requested = transaction.status == PaymentStatusEnum::Cancelled;
        if current.status == StripePaymentStatus::RequiresConfirmation && !cancel_requested {
            // The pinned id prevents a second successful charge, so each attempt may use a fresh
            // key; an error reply is resolved by re-reading the intent, never by guessing.
            current = match client
                .confirm_payment_intent(
                    id,
                    ConfirmPaymentIntentRequest {
                        off_session: Some(!on_session),
                        use_stripe_sdk: on_session.then_some(true),
                    },
                    &secret,
                    format!(
                        "wallet_confirm:{}:{}",
                        transaction.id.as_base62(),
                        Uuid::new_v4()
                    ),
                )
                .await
            {
                Ok(intent) => intent,
                Err(_) => client
                    .get_payment_intent(id, &secret)
                    .await
                    .map_err(map_stripe_error)?,
            };
            self.validate_prepared(&current, transaction)?;
        }
        let cannot_continue = (cancel_requested
            && !matches!(
                current.status,
                StripePaymentStatus::Succeeded | StripePaymentStatus::Canceled
            ))
            || matches!(
                current.status,
                StripePaymentStatus::Failed
                    | StripePaymentStatus::RequiresPaymentMethod
                    | StripePaymentStatus::RequiresCapture
            )
            || (!on_session && current.status == StripePaymentStatus::RequiresCustomerAction);
        // A decline cancels the intent so nothing can charge it later; the ledger still sees a decline.
        let mut declined = None;
        if cannot_continue {
            let previous_error = current.last_payment_error.clone();
            let awaiting_authentication = previous_error.is_none()
                && current.status == StripePaymentStatus::RequiresCustomerAction;
            current = match client
                .cancel_payment_intent(
                    id,
                    CancelPaymentIntentRequest {
                        cancellation_reason: "abandoned".to_owned(),
                    },
                    &secret,
                    format!(
                        "wallet_cancel:{}:{}",
                        transaction.id.as_base62(),
                        Uuid::new_v4()
                    ),
                )
                .await
            {
                Ok(intent) => intent,
                Err(_) => client
                    .get_payment_intent(id, &secret)
                    .await
                    .map_err(map_stripe_error)?,
            };
            self.validate_prepared(&current, transaction)?;
            if !matches!(
                current.status,
                StripePaymentStatus::Canceled | StripePaymentStatus::Succeeded
            ) {
                return Err(mismatch("the top-up payment could not be cancelled yet"));
            }
            if current.status == StripePaymentStatus::Canceled
                && !cancel_requested
                && (previous_error.is_some() || awaiting_authentication)
            {
                declined = Some(ChargeFailure {
                    external_id: Some(current.id.clone()),
                    code: stripe_error_code(&previous_error),
                    message: flatten_payment_error(&previous_error).unwrap_or_else(|| {
                        "authentication_required: payment requires customer authentication"
                            .to_owned()
                    }),
                    retryable: false,
                    decline_kind: if awaiting_authentication {
                        DeclineKind::AuthenticationRequired
                    } else {
                        decline_kind_from_error(&previous_error)
                    },
                    provider_request_id: None,
                });
            }
            // A cancellation can race a successful confirmation: that success is kept and funded.
            if current.status == StripePaymentStatus::Canceled
                && current.last_payment_error.is_none()
            {
                current.last_payment_error = previous_error;
            }
        }
        if current.status == StripePaymentStatus::RequiresConfirmation {
            return Err(mismatch("the top-up payment confirmation is unresolved"));
        }
        if let Some(failure) = declined {
            return Ok(ChargeOutcome::Failed(failure));
        }
        Ok(intent_to_outcome(current, &self.public.api_publishable_key))
    }
}
