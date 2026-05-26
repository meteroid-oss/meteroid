//! GoCardless Billing Request Flow return-URL handler.
//!
//! When a customer completes the GC-hosted mandate consent, GoCardless
//! redirects the browser to the `redirect_uri` we provided when minting the
//! Billing Request Flow. The REST endpoint in `src/api_rest/portal/` calls
//! into this service:
//!
//! 1. Look up the connection (by id) and its connector / tenant.
//! 2. Call `MandateOps::complete_mandate_setup` on the GoCardless adapter,
//!    which (a) completes the Billing Request and (b) fetches the resulting
//!    mandate, returning a snapshot.
//! 3. Upsert as a [`CustomerPaymentMethod`] for the connection and set it
//!    as the customer's current payment method.
//!
//! The `mandates.active` webhook also handles step 3 idempotently — this
//! handler is mostly UX (instant confirmation rather than waiting for the
//! webhook to arrive 30–60s later).

use crate::StoreResult;
use crate::adapters::payment::initialize_payment_connector;
use crate::domain::connectors::Connector;
use crate::domain::{
    CustomerPatch, CustomerPaymentMethod, CustomerPaymentMethodNew, PaymentMethodTypeEnum,
};
use crate::errors::StoreError;
use crate::repositories::CustomersInterface;
use crate::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use crate::services::Services;
use common_domain::ids::{BaseId, CustomerConnectionId, CustomerPaymentMethodId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use error_stack::{Report, ResultExt};

impl Services {
    /// Finalize a GoCardless Billing Request after the customer returns from
    /// the hosted authorisation flow.
    ///
    /// Idempotent: the underlying GC `complete_billing_request` call can be
    /// retried safely, and `upsert_payment_method` dedups on
    /// `(connection_id, external_payment_method_id)`. Calling this from the
    /// return-URL handler AND processing the mandates.active webhook on the
    /// same mandate yields the same end state.
    ///
    /// The tenant id is *derived from* the connection — the return-URL
    /// handler is hit by a customer's browser after a third-party redirect,
    /// so there's no authenticated tenant context. Because `connection_id` is
    /// attacker-supplied, we do not trust it: we verify that the Billing
    /// Request's metadata (set by us at creation, returned by the provider on
    /// completion) names exactly this connection + customer before attaching
    /// the mandate, and fail closed otherwise.
    pub async fn complete_gocardless_setup(
        &self,
        connection_id: CustomerConnectionId,
        billing_request_id: String,
    ) -> StoreResult<CustomerPaymentMethod> {
        let mut conn = self.store.get_conn().await?;

        let connection_row =
            CustomerConnectionDetailsRow::get_by_id_unscoped(&mut conn, &connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let connector =
            Connector::from_row(&self.store.settings.crypt_key, connection_row.connector)?;

        let tenant_id = connector.tenant_id;

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        let snapshot = connector_impl
            .complete_mandate_setup(&connector, &billing_request_id)
            .await
            .change_context_lazy(|| StoreError::PaymentProviderError)?;

        // Authorization check (defense against mandate hijack). This endpoint is
        // unauthenticated — it is hit by the customer's browser after a
        // third-party redirect — and `connection_id` comes straight from the
        // query string. Without this check, anyone who learns a victim's
        // connection id (it appears in URLs / logs) could complete *their own*
        // Billing Request against the victim's connection and have it attached
        // to the victim's customer record. We verify that the metadata stamped
        // onto the Billing Request at creation (which the provider returns on
        // completion) names exactly this connection and customer. Fail closed if
        // it is missing or mismatched. This mirrors the cross-tenant defense the
        // `mandates.active` webhook handler applies.
        let expected_connection = connection_id.as_base62();
        let expected_customer = connection_row.customer.id.as_base62();
        match (
            snapshot.meteroid_connection_id.as_deref(),
            snapshot.meteroid_customer_id.as_deref(),
        ) {
            (Some(conn), Some(cust))
                if conn == expected_connection && cust == expected_customer => {}
            other => {
                return Err(Report::new(StoreError::InvalidArgument(
                    "GoCardless billing request does not belong to this connection".to_string(),
                ))
                .attach(format!(
                    "expected connection={expected_connection} customer={expected_customer}, \
                     billing request carried {other:?}"
                )));
            }
        }

        // Force the payment method type to direct-debit-* even if the
        // snapshot says Other (which happens when the mandate's scheme isn't
        // one we recognise yet). Mandates with unknown schemes still belong
        // to a direct-debit conceptual bucket.
        let payment_method_type = match snapshot.payment_method_type {
            PaymentMethodTypeEnum::Other => PaymentMethodTypeEnum::DirectDebitSepa,
            other => other,
        };

        let payment_method = self
            .store
            .upsert_payment_method(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id,
                customer_id: connection_row.customer.id,
                connection_id,
                external_payment_method_id: snapshot.external_payment_method_id,
                payment_method_type,
                account_number_hint: snapshot.account_number_hint,
                card_brand: snapshot.card_brand,
                card_last4: snapshot.card_last4,
                card_exp_month: snapshot.card_exp_month,
                card_exp_year: snapshot.card_exp_year,
            })
            .await?;

        let patch = CustomerPatch {
            id: connection_row.customer.id,
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
            .patch_customer(connection_row.customer.id.as_uuid(), tenant_id, patch)
            .await?;

        Ok(payment_method)
    }
}
