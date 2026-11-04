//! Return-URL completion for webhook-backed hosted setups that carry an intent id (Mollie).
//! Attaches the existing mandate (the default when the customer has none), as the webhook does.
//! No money moves here, so a lost or replayed return is harmless.

use crate::StoreResult;
use crate::adapters::payment::error::{
    ConnectorError, HostedSetupNotCompleted, HostedSetupPending,
};
use crate::adapters::payment::initialize_payment_connector;
use crate::domain::connectors::Connector;
use crate::domain::entity_activity::Actor;
use crate::domain::{CustomerPaymentMethod, CustomerPaymentMethodNew};
use crate::errors::StoreError;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::services::Services;
use common_domain::ids::{BaseId, CustomerPaymentMethodId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use error_stack::{Report, ResultExt};
use std::time::Duration;

const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

/// Customer-facing outcome of a webhook-backed hosted-setup return.
#[derive(Debug)]
pub enum WebhookBackedSetupOutcome {
    /// The mandate exists and is saved on the customer.
    MethodSaved(Box<CustomerPaymentMethod>),
    /// Not final at the provider yet; the webhook will complete it.
    Processing,
    /// The customer cancelled on the hosted page (Mollie `open`); can resume.
    NotCompleted,
    /// No mandate; carries the connector so the caller can apply the final state.
    SetupFailed(Box<Connector>),
}

impl Services {
    /// Unauthenticated and the connection comes from the caller: the payment metadata must match
    /// this connection and customer.
    pub(crate) async fn complete_webhook_backed_setup(
        &self,
        connection_row: CustomerConnectionDetailsRow,
        intent_id: String,
    ) -> StoreResult<WebhookBackedSetupOutcome> {
        let connection_id = connection_row.id;
        let connector =
            Connector::from_row(&self.store.settings.crypt_key, connection_row.connector)?;
        let webhook_backed = crate::adapters::payment::payment_capabilities(&connector.provider())
            .is_some_and(|caps| caps.is_webhook_backed_hosted_setup());
        if !webhook_backed {
            return Err(Report::new(StoreError::InvalidArgument(
                "connection's provider does not use webhook-backed hosted-setup completion"
                    .to_string(),
            )));
        }

        let tenant_id = connector.tenant_id;
        let customer_id = connection_row.customer.id;
        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let result = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.complete_mandate_setup(&intent_id),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?;

        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(report) => {
                let pending = report
                    .frames()
                    .any(|f| f.downcast_ref::<HostedSetupPending>().is_some());
                let not_completed = report
                    .frames()
                    .any(|f| f.downcast_ref::<HostedSetupNotCompleted>().is_some());
                return if pending && not_completed {
                    Ok(WebhookBackedSetupOutcome::NotCompleted)
                } else if pending {
                    Ok(WebhookBackedSetupOutcome::Processing)
                } else if matches!(report.current_context(), ConnectorError::MandateSetup(_)) {
                    log::info!("hosted setup for intent {intent_id} did not complete: {report:?}");
                    Ok(WebhookBackedSetupOutcome::SetupFailed(Box::new(
                        connector.clone(),
                    )))
                } else {
                    Err(report.change_context(StoreError::PaymentProviderError))
                };
            }
        };

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

        let in_flow_invoice = snapshot.meteroid_invoice_id.clone();
        let in_flow_transaction = snapshot.meteroid_transaction_id.clone();
        let in_flow_captured = snapshot.payment_request_payment.clone();
        let pays_checkout = snapshot.meteroid_checkout_session_id.is_some();

        let payment_method = self
            .store
            .upsert_payment_method(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id,
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
                saved_for_payment: in_flow_invoice.is_some() || pays_checkout,
            })
            .await?;

        // A setup the customer made on its own is their new default; one that pays something
        // becomes it once that payment settles.
        if in_flow_invoice.is_none() && !pays_checkout {
            self.adopt_default_payment_method(
                Actor::Customer { id: customer_id },
                tenant_id,
                customer_id,
                payment_method.id,
                super::hosted_setup::DefaultMethodAdoption::Replace,
            )
            .await?;
        }

        // Record the invoice capture now so it shows as paid on return; the webhook repeats this.
        if let (Some(invoice_id_str), Some(_)) = (&in_flow_invoice, &in_flow_transaction) {
            let settled = self
                .settle_hosted_invoice_capture(
                    tenant_id,
                    customer_id,
                    &connector,
                    payment_method.clone(),
                    invoice_id_str,
                    in_flow_captured,
                    in_flow_transaction,
                )
                .await;
            if let Err(e) = settled {
                log::warn!(
                    "hosted invoice {invoice_id_str} capture not recorded on return (webhook will): {e:?}"
                );
            }
        }

        Ok(WebhookBackedSetupOutcome::MethodSaved(Box::new(
            payment_method,
        )))
    }
}
