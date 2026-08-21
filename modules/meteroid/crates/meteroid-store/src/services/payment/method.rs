use crate::StoreResult;
use crate::adapters::payment::initialize_payment_connector;
use crate::adapters::payment::model::{
    CreateCustomerRequest, IdempotencyKey, MandateSetupInstruction, MandateSetupRequest,
};
use crate::domain::connectors::Connector;
use crate::domain::{CustomerConnection, PaymentMethodTypeEnum, SetupIntent};
use crate::errors::StoreError;
use crate::repositories::customers::CustomersInterfaceAuto;
use crate::services::Services;
use crate::services::payment::sepa::SEPA_COUNTRIES;
use crate::store::PgConn;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomerConnectionId, InvoiceId, TenantId};
use diesel_models::customer_connection::CustomerConnectionDetailsRow;
use diesel_models::invoicing_entities::{InvoicingEntityProvidersRow, InvoicingEntityRow};
use error_stack::{Report, ResultExt};
use secrecy::SecretString;
use std::time::Duration;

/// Build a CreateCustomerRequest with an idempotency key derived from
/// (customer, provider). Stable across retries; unique per (customer, provider)
/// pair so two providers don't collide.
fn customer_idempotency(
    customer_id: common_domain::ids::CustomerId,
    provider_id: common_domain::ids::ConnectorId,
) -> CreateCustomerRequest {
    CreateCustomerRequest {
        idempotency_key: IdempotencyKey::new(format!(
            "customer:{}:{}",
            customer_id.as_base62(),
            provider_id.as_base62()
        )),
    }
}

/// Maximum time to wait for payment provider API calls.
const PAYMENT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

/// Helper function to determine which direct debit payment methods are supported
/// based on the invoicing entity's country
fn get_direct_debit_types_for_country(
    country: &CountryCode,
) -> Vec<Option<diesel_models::enums::PaymentMethodTypeEnum>> {
    let Some(iso_country_code) = rust_iso3166::from_alpha2(&country.code) else {
        log::warn!("Invalid country code: {}", country.code);
        return vec![];
    };

    if iso_country_code == rust_iso3166::US || iso_country_code == rust_iso3166::CA {
        vec![Some(
            diesel_models::enums::PaymentMethodTypeEnum::DirectDebitAch,
        )]
    } else if SEPA_COUNTRIES.contains(&iso_country_code) {
        vec![Some(
            diesel_models::enums::PaymentMethodTypeEnum::DirectDebitSepa,
        )]
    } else {
        vec![]
    }
}

impl Services {
    /// Gets existing or creates new customer connections for card and direct debit providers
    /// This ensures customers can add payment methods even if they don't have connections yet
    pub(in crate::services) async fn get_or_create_customer_connections(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: common_domain::ids::CustomerId,
        invoicing_entity_id: common_domain::ids::InvoicingEntityId,
    ) -> StoreResult<(Option<CustomerConnectionId>, Option<CustomerConnectionId>)> {
        use diesel_models::customer_connection::CustomerConnectionRow;

        let customer = self
            .store
            .find_customer_by_id(customer_id, tenant_id)
            .await?;

        let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            conn,
            invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let providers = diesel_models::invoicing_entities::InvoicingEntityProvidersRow::resolve_providers_by_id(
            conn,
            invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let providers_sensitive = crate::domain::InvoicingEntityProviderSensitive::from_row(
            providers,
            &self.store.settings.crypt_key,
        )?;

        let existing_connections = diesel_models::customer_connection::CustomerConnectionRow::list_connections_by_customer_id(
            conn,
            &tenant_id,
            &customer_id,
        )
        .await
        .map_err(|err| StoreError::DatabaseError(err.error))?;

        let mut card_connection_id = None;
        let mut direct_debit_connection_id = None;

        // Check if the same provider is used for both card and direct debit
        let same_provider = match (
            &providers_sensitive.card_provider,
            &providers_sensitive.direct_debit_provider,
        ) {
            (Some(card), Some(dd)) => card.id == dd.id,
            _ => false,
        };

        if same_provider {
            // Same provider handles both card and direct debit
            // Create a single connection with combined payment types
            let provider = providers_sensitive.card_provider.as_ref().unwrap();

            let existing = existing_connections
                .iter()
                .find(|c| c.connector_id == provider.id);

            let connection_id = if let Some(conn_row) = existing {
                conn_row.id
            } else {
                // Create new customer in payment provider
                let connector_impl = initialize_payment_connector(provider)
                    .change_context(StoreError::PaymentProviderError)?;

                let external_ref = tokio::time::timeout(
                    PAYMENT_PROVIDER_TIMEOUT,
                    connector_impl.create_customer(
                        provider,
                        &customer,
                        customer_idempotency(customer.id, provider.id),
                    ),
                )
                .await
                .map_err(|_| {
                    Report::new(StoreError::PaymentProviderError)
                        .attach("Payment provider request timed out")
                })?
                .change_context(StoreError::PaymentProviderError)?;

                let external_id = external_ref.external_id;

                // Combine payment types: Card + appropriate direct debit types based on country
                let mut payment_types =
                    vec![Some(diesel_models::enums::PaymentMethodTypeEnum::Card)];
                payment_types.extend(get_direct_debit_types_for_country(
                    &invoicing_entity.country,
                ));

                let new_connection = CustomerConnectionRow {
                    id: CustomerConnectionId::new(),
                    customer_id,
                    connector_id: provider.id,
                    external_customer_id: external_id,
                    supported_payment_types: Some(payment_types),
                };

                let inserted = new_connection
                    .insert(conn)
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;

                inserted.id
            };

            // Use the same connection ID for both
            card_connection_id = Some(connection_id);
            direct_debit_connection_id = Some(connection_id);
        } else {
            // Different providers for card and direct debit - create separate connections
            if let Some(card_provider) = &providers_sensitive.card_provider {
                let existing = existing_connections
                    .iter()
                    .find(|c| c.connector_id == card_provider.id);

                if let Some(conn_row) = existing {
                    card_connection_id = Some(conn_row.id);
                } else {
                    let connector_impl = initialize_payment_connector(card_provider)
                        .change_context(StoreError::PaymentProviderError)?;

                    let external_ref = tokio::time::timeout(
                        PAYMENT_PROVIDER_TIMEOUT,
                        connector_impl.create_customer(
                            card_provider,
                            &customer,
                            customer_idempotency(customer.id, card_provider.id),
                        ),
                    )
                    .await
                    .map_err(|_| {
                        Report::new(StoreError::PaymentProviderError)
                            .attach("Payment provider request timed out")
                    })?
                    .change_context(StoreError::PaymentProviderError)?;

                    let external_id = external_ref.external_id;

                    // Create connection in our database with Card payment type
                    let new_connection = CustomerConnectionRow {
                        id: CustomerConnectionId::new(),
                        customer_id,
                        connector_id: card_provider.id,
                        external_customer_id: external_id,
                        supported_payment_types: Some(vec![Some(
                            diesel_models::enums::PaymentMethodTypeEnum::Card,
                        )]),
                    };

                    let inserted = new_connection
                        .insert(conn)
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    card_connection_id = Some(inserted.id);
                }
            }

            // Check for direct debit provider connection
            if let Some(direct_debit_provider) = &providers_sensitive.direct_debit_provider {
                let existing = existing_connections
                    .iter()
                    .find(|c| c.connector_id == direct_debit_provider.id);

                if let Some(conn_row) = existing {
                    direct_debit_connection_id = Some(conn_row.id);
                } else {
                    let connector_impl = initialize_payment_connector(direct_debit_provider)
                        .change_context(StoreError::PaymentProviderError)?;

                    let external_ref = tokio::time::timeout(
                        PAYMENT_PROVIDER_TIMEOUT,
                        connector_impl.create_customer(
                            direct_debit_provider,
                            &customer,
                            customer_idempotency(customer.id, direct_debit_provider.id),
                        ),
                    )
                    .await
                    .map_err(|_| {
                        Report::new(StoreError::PaymentProviderError)
                            .attach("Payment provider request timed out")
                    })?
                    .change_context(StoreError::PaymentProviderError)?;

                    let external_id = external_ref.external_id;

                    let new_connection = CustomerConnectionRow {
                        id: CustomerConnectionId::new(),
                        customer_id,
                        connector_id: direct_debit_provider.id,
                        external_customer_id: external_id,
                        supported_payment_types: Some(get_direct_debit_types_for_country(
                            &invoicing_entity.country,
                        )),
                    };

                    let inserted = new_connection
                        .insert(conn)
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    direct_debit_connection_id = Some(inserted.id);
                }
            }
        }

        Ok((card_connection_id, direct_debit_connection_id))
    }

    pub(in crate::services) async fn create_setup_intent_for_type(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        connection_type: crate::domain::ConnectionTypeEnum,
        return_url: Option<String>,
    ) -> StoreResult<SetupIntent> {
        self.create_setup_intent_internal(
            conn,
            tenant_id,
            customer_connection_id,
            Some(connection_type),
            // "Add a payment method" flow — not tied to an invoice.
            None,
            None,
            return_url,
        )
        .await
    }

    /// Set up a mandate AND collect the first payment in a single hosted flow, for
    /// a checkout on a hosted-redirect provider (GoCardless). Returns the intent
    /// carrying the Billing Request id (`intent_id`) and the hosted
    /// `authorisation_url` (in `client_secret`). The caller pre-creates the Pending
    /// checkout transaction whose id is in `checkout.transaction_id`.
    pub(in crate::services) async fn create_hosted_checkout_intent(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        checkout: crate::adapters::payment::model::HostedCheckoutContext,
        return_url: Option<String>,
    ) -> StoreResult<SetupIntent> {
        self.create_setup_intent_internal(
            conn,
            tenant_id,
            customer_connection_id,
            Some(crate::domain::ConnectionTypeEnum::DirectDebit),
            None,
            Some(checkout),
            return_url,
        )
        .await
    }

    pub(in crate::services) async fn create_setup_intent(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        invoice_id: Option<InvoiceId>,
        return_url: Option<String>,
    ) -> StoreResult<SetupIntent> {
        self.create_setup_intent_internal(
            conn,
            tenant_id,
            customer_connection_id,
            None,
            invoice_id,
            None,
            return_url,
        )
        .await
    }

    async fn create_setup_intent_internal(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        customer_connection_id: &CustomerConnectionId,
        requested_connection_type: Option<crate::domain::ConnectionTypeEnum>,
        // When set, this setup is paying a specific invoice; for hosted-redirect
        // providers it's stored in the Billing Request metadata so the
        // `billing_requests.fulfilled` webhook can charge it once the mandate exists.
        invoice_id: Option<InvoiceId>,
        // When set, this is a hosted CHECKOUT: the provider collects the first
        // payment together with the mandate in one hosted flow (GoCardless
        // combined mandate+payment Billing Request). Mutually exclusive with
        // `invoice_id`.
        checkout: Option<crate::adapters::payment::model::HostedCheckoutContext>,
        return_url: Option<String>,
    ) -> StoreResult<SetupIntent> {
        let connection =
            CustomerConnectionDetailsRow::get_by_id(conn, tenant_id, customer_connection_id)
                .await
                .map_err(|err| StoreError::DatabaseError(err.error))?;

        let customer_connection: CustomerConnection = CustomerConnection {
            id: connection.id,
            customer_id: connection.customer.id,
            connector_id: connection.connector.id,
            supported_payment_types: connection
                .supported_payment_types
                .as_ref()
                .map(|v| v.iter().flatten().map(|t| t.clone().into()).collect()),
            external_customer_id: connection.external_customer_id,
        };

        let connector = Connector::from_row(&self.store.settings.crypt_key, connection.connector)?;

        // Combined mandate+payment hosted checkout is GoCardless-only (Mock is
        // admitted as the integration-test stand-in); other providers would
        // silently ignore the ctx and return a non-URL secret.
        if checkout.is_some()
            && !matches!(
                connector.provider,
                crate::domain::enums::ConnectorProviderEnum::Gocardless
                    | crate::domain::enums::ConnectorProviderEnum::Mock
            )
        {
            return Err(error_stack::Report::new(StoreError::InvalidArgument(
                "Hosted checkout is only supported for hosted-redirect (GoCardless) direct-debit connections".to_string(),
            )));
        }

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)?;

        // payment methods for that connector are either retrieved from invoicing entity (default) or overridden through the connection
        let mut payment_methods = match connection.supported_payment_types {
            Some(types) if !types.is_empty() => types
                .into_iter()
                .filter_map(|t| t.map(Into::<PaymentMethodTypeEnum>::into))
                .collect(),
            _ => {
                let invoicing_entity_providers =
                    InvoicingEntityProvidersRow::resolve_providers_by_id(
                        conn,
                        connection.customer.invoicing_entity_id,
                        *tenant_id,
                    )
                    .await
                    .map_err(|err| StoreError::DatabaseError(err.error))?;

                let mut payment_methods = Vec::new();
                if let Some(card_provider) = invoicing_entity_providers.card_provider
                    && card_provider.id == connector.id
                {
                    payment_methods.push(PaymentMethodTypeEnum::Card);
                }
                if let Some(direct_debit_provider) =
                    invoicing_entity_providers.direct_debit_provider
                    && direct_debit_provider.id == connector.id
                {
                    let invoicing_entity =
                        InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                            conn,
                            connection.customer.invoicing_entity_id,
                            *tenant_id,
                        )
                        .await
                        .map_err(|err| StoreError::DatabaseError(err.error))?;

                    // Use the helper function to determine direct debit types based on country
                    let direct_debit_types =
                        get_direct_debit_types_for_country(&invoicing_entity.country);
                    payment_methods.extend(
                        direct_debit_types
                            .into_iter()
                            .filter_map(|t| t.map(Into::<PaymentMethodTypeEnum>::into)),
                    );
                }

                payment_methods
            }
        };

        // Filter payment methods based on requested connection type if specified
        if let Some(requested_type) = requested_connection_type {
            payment_methods.retain(|pm| match requested_type {
                crate::domain::ConnectionTypeEnum::Card => {
                    matches!(pm, PaymentMethodTypeEnum::Card)
                }
                crate::domain::ConnectionTypeEnum::DirectDebit => matches!(
                    pm,
                    PaymentMethodTypeEnum::DirectDebitSepa
                        | PaymentMethodTypeEnum::DirectDebitAch
                        | PaymentMethodTypeEnum::DirectDebitBacs
                ),
            });
        }

        // GoCardless must redirect back to our backend completion endpoint (to
        // finalize the BR), not a client-supplied URL. Build it server-side
        // from the REST API's external base URL. The customer's desired
        // post-flow page rides along as a validated `dest` query param
        // (same-origin as our public URL, or dropped); the return handler
        // bounces there once the mandate is stored. The adapter uses this same
        // value for both `redirect_uri` and `exit_uri`, so an abandoned flow
        // lands on the handler too (without a `billing_request`), and the
        // handler treats that as "abandoned".
        let return_url =
            if connector.provider == crate::domain::enums::ConnectorProviderEnum::Gocardless {
                let handler_url = format!(
                    "{}/v1/portal/gocardless/return?connection={}",
                    self.store
                        .settings
                        .rest_api_external_url
                        .trim_end_matches('/'),
                    customer_connection.id.as_base62(),
                );
                match return_url.as_deref() {
                    Some(target) if same_origin(&self.store.settings.public_url, target) => Some(
                        format!("{handler_url}&dest={}", urlencoding::encode(target)),
                    ),
                    _ => Some(handler_url),
                }
            } else {
                return_url
            };

        // A GoCardless idempotency key protects ONE creation attempt against
        // automatic retries (it 409s `idempotent_creation_conflict` for ~30 days
        // otherwise). It must NOT be reused across separate user-initiated
        // attempts — a stable `setup_intent:{connection}` key meant that clicking
        // "set up direct debit" a second time (e.g. after going back) collided and
        // failed. Mint a fresh key per attempt so each start creates a new Billing
        // Request + hosted Flow (the Flow is single-use anyway); the key still
        // stays fixed across the client's internal retries within this one call.
        let mandate_request = MandateSetupRequest {
            payment_methods: &payment_methods,
            idempotency_key: IdempotencyKey::new(format!(
                "setup_intent:{}:{}",
                customer_connection.id.as_base62(),
                uuid::Uuid::now_v7().simple()
            )),
            return_url,
            // Carried into the mandate metadata so a hosted-redirect provider's
            // mandate-active webhook can charge this invoice after setup.
            invoice_id: invoice_id.map(|id| id.as_base62()),
            // Present for a hosted checkout: adds a `payment_request` so the first
            // payment is collected in the same hosted flow as the mandate.
            checkout,
        };

        let instruction = tokio::time::timeout(
            PAYMENT_PROVIDER_TIMEOUT,
            connector_impl.initiate_mandate_setup(
                &connector,
                &customer_connection,
                mandate_request,
            ),
        )
        .await
        .map_err(|_| {
            Report::new(StoreError::PaymentProviderError)
                .attach("Payment provider request timed out")
        })?
        .change_context_lazy(|| StoreError::PaymentProviderError)?;

        let setup_intent = match instruction {
            MandateSetupInstruction::EmbeddedClientSecret {
                intent_id,
                client_secret,
                publishable_key,
            } => SetupIntent {
                intent_id,
                client_secret,
                public_key: publishable_key,
                provider: connector.provider.clone(),
                connector_id: connector.id,
                connection_id: customer_connection.id,
            },
            // GoCardless / Adyen Drop-in surfaces happen here. Today's
            // frontend only consumes the embedded-client-secret shape — the
            // SetupIntent gRPC response is mapped accordingly. When we add
            // GoCardless (Phase 2) we'll widen SetupIntent or split the API.
            MandateSetupInstruction::HostedRedirect {
                intent_id,
                authorisation_url,
                ..
            } => SetupIntent {
                intent_id,
                client_secret: authorisation_url,
                public_key: SecretString::from(String::new()),
                provider: connector.provider.clone(),
                connector_id: connector.id,
                connection_id: customer_connection.id,
            },
            MandateSetupInstruction::EmbeddedDropIn {
                intent_id,
                session_data,
                ..
            } => SetupIntent {
                intent_id,
                client_secret: session_data,
                public_key: SecretString::from(String::new()),
                provider: connector.provider.clone(),
                connector_id: connector.id,
                connection_id: customer_connection.id,
            },
        };

        Ok(setup_intent)
    }
}

/// Same-origin gate for the customer-supplied post-redirect target before we
/// reflect it into the GoCardless return-handler URL. Scheme + host + port must
/// match the configured public URL exactly; anything unparseable, cross-origin,
/// or protocol-relative fails closed so we never hand GoCardless (or, later,
/// our own 302) an attacker-chosen origin.
fn same_origin(configured: &str, candidate: &str) -> bool {
    match (url_origin(configured), url_origin(candidate)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Extract `scheme://authority` (lower-cased) from a URL string, or `None` when
/// it isn't a parseable absolute URL. The authority ends at the first `/`, `?`
/// or `#`, so userinfo/path tricks can't smuggle a different host past the
/// comparison.
fn url_origin(raw: &str) -> Option<String> {
    let (scheme, rest) = raw.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::same_origin;

    const PUBLIC: &str = "https://billing.example.com";

    #[test]
    fn accepts_same_origin_targets() {
        assert!(same_origin(
            PUBLIC,
            "https://billing.example.com/checkout?token=abc"
        ));
        assert!(same_origin(
            PUBLIC,
            "https://Billing.Example.com/portal/customer"
        ));
        assert!(same_origin(
            "https://billing.example.com/",
            "https://billing.example.com/portal/invoice-payment/i_1?token=x"
        ));
    }

    #[test]
    fn rejects_cross_origin_and_tricks() {
        // Different host.
        assert!(!same_origin(PUBLIC, "https://evil.com/checkout"));
        // Suffix host.
        assert!(!same_origin(
            PUBLIC,
            "https://billing.example.com.evil.com/"
        ));
        // Userinfo trick — real host is evil.com.
        assert!(!same_origin(
            PUBLIC,
            "https://billing.example.com@evil.com/"
        ));
        // Scheme mismatch.
        assert!(!same_origin(PUBLIC, "http://billing.example.com/"));
        // Not absolute / protocol-relative.
        assert!(!same_origin(PUBLIC, "//billing.example.com/checkout"));
        assert!(!same_origin(PUBLIC, "/checkout?token=abc"));
        assert!(!same_origin(PUBLIC, "javascript:alert(1)"));
    }
}
