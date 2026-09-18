//! Provider-agnostic completion of a hosted-redirect return. Picks the completion routine from
//! [`HostedSetupCompletion`] and maps both result types to one outcome enum shared with the
//! frontend.

use crate::StoreResult;
use crate::adapters::payment::{HostedSetupCompletion, provider_capabilities};
use crate::domain::connectors::Connector;
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::payment::hosted_setup::{COMPLETE_ATTEMPTS, HostedSetupOutcome};
use crate::services::payment::webhook_backed_setup::WebhookBackedSetupOutcome;
use common_domain::ids::CustomerConnectionId;
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use error_stack::Report;

/// Customer-facing outcome of a hosted return, ordered as the frontend handles them.
#[derive(Debug)]
pub enum HostedReturnOutcome {
    /// Method saved, and the invoice or checkout (if any) is paid.
    Completed,
    /// Not final at the provider yet, or captured but held for review. Never offer a retry.
    Processing,
    /// Method saved but the first charge was declined; can retry with the saved method.
    PaymentFailed,
    /// No method saved. `connector` is set for webhook-backed providers so the caller can re-read
    /// the intent and apply its final state now.
    SetupFailed { connector: Option<Box<Connector>> },
    /// The customer cancelled on the hosted page; can resume.
    Abandoned,
    /// The return URL is missing the intent id this provider expects.
    MissingIntent,
}

impl Services {
    /// Unauthenticated, and `connection_id` comes from the caller: both routines check the intent's
    /// metadata before attaching anything. Without an intent id in the URL the return is only
    /// classified; the webhook completes the setup.
    pub async fn complete_hosted_return(
        &self,
        connection_id: CustomerConnectionId,
        intent_id: Option<String>,
    ) -> StoreResult<HostedReturnOutcome> {
        let mut conn = self.store.get_conn().await?;
        let connection_row =
            CustomerConnectionDetailsRow::get_by_id_unscoped(&mut conn, &connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;
        drop(conn);
        let provider = connection_row.connector.provider.clone().into();

        let caps = provider_capabilities(&provider)
            .filter(|caps| caps.is_hosted_redirect())
            .ok_or_else(|| {
                Report::new(StoreError::InvalidArgument(
                    "connection's provider has no hosted-redirect flow".to_string(),
                ))
            })?;

        let intent_id = match (caps.completes_pending_hosted_intents(), intent_id) {
            (true, Some(id)) => id,
            (true, None) => return Ok(HostedReturnOutcome::MissingIntent),
            (false, _) => return Ok(HostedReturnOutcome::Completed),
        };

        Ok(match caps.hosted_setup_completion {
            HostedSetupCompletion::PollingRequired => {
                match self
                    .complete_hosted_setup_for_connection(
                        connection_row,
                        intent_id,
                        COMPLETE_ATTEMPTS,
                    )
                    .await?
                {
                    HostedSetupOutcome::MethodSaved(_)
                    | HostedSetupOutcome::InvoiceCharged(_)
                    | HostedSetupOutcome::CheckoutActivated(_) => HostedReturnOutcome::Completed,
                    HostedSetupOutcome::Processing => HostedReturnOutcome::Processing,
                    HostedSetupOutcome::PaymentFailed { .. } => HostedReturnOutcome::PaymentFailed,
                    HostedSetupOutcome::SetupFailed { .. } => {
                        HostedReturnOutcome::SetupFailed { connector: None }
                    }
                    // Money was captured but doesn't reconcile: a retry would double-charge.
                    HostedSetupOutcome::HeldForReview { .. } => {
                        log::error!(
                            "Hosted setup for connection {connection_id}: captured payment held for manual review"
                        );
                        HostedReturnOutcome::Processing
                    }
                }
            }
            HostedSetupCompletion::WebhookBacked => {
                match self
                    .complete_webhook_backed_setup(connection_row, intent_id)
                    .await?
                {
                    WebhookBackedSetupOutcome::MethodSaved(_) => HostedReturnOutcome::Completed,
                    WebhookBackedSetupOutcome::Processing => HostedReturnOutcome::Processing,
                    WebhookBackedSetupOutcome::NotCompleted => HostedReturnOutcome::Abandoned,
                    WebhookBackedSetupOutcome::SetupFailed(connector) => {
                        HostedReturnOutcome::SetupFailed {
                            connector: Some(connector),
                        }
                    }
                }
            }
        })
    }
}
