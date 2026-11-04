use crate::connectors::mapping::connectors::connector_provider_to_server;
use crate::customers::error::CustomerApiError;
use crate::customers::mapping::customer::{
    DomainAddressWrapper, DomainShippingAddressWrapper, ServerCustomerWrapper,
};
use crate::portal::shared::PortalSharedServiceComponents;
use crate::portal::shared::error::PortalSharedApiError;
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, InvoiceId,
};
use common_grpc::middleware::server::auth::{AuthorizedAsPortalUser, RequestExt};
use error_stack::ResultExt;
use meteroid_grpc::meteroid::portal::shared::v1::portal_shared_service_server::PortalSharedService;
use meteroid_grpc::meteroid::portal::shared::v1::*;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::{CustomerPatch, CustomerPaymentMethodNew};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::{CustomersInterface, CustomersInterfaceAuto};
use secrecy::ExposeSecret;
use tonic::{Request, Response, Status};

impl PortalSharedServiceComponents {
    async fn resolve_customer(
        &self,
        resource: AuthorizedAsPortalUser,
    ) -> Result<CustomerId, PortalSharedApiError> {
        match resource.resource_access {
            common_grpc::middleware::server::auth::ResourceAccess::CheckoutSession(session_id) => {
                let session = self
                    .store
                    .get_checkout_session(resource.tenant_id, session_id)
                    .await
                    .map_err(Into::<PortalSharedApiError>::into)?;

                Ok(session.customer_id)
            }
            common_grpc::middleware::server::auth::ResourceAccess::InvoicePortal(invoice_id) => {
                let invoice = self
                    .store
                    .get_invoice_by_id(resource.tenant_id, invoice_id)
                    .await
                    .map_err(Into::<PortalSharedApiError>::into)?;

                Ok(invoice.customer_id)
            }
            common_grpc::middleware::server::auth::ResourceAccess::CustomerPortal(id) => Ok(id),
            common_grpc::middleware::server::auth::ResourceAccess::QuotePortal { .. } => {
                Err(PortalSharedApiError::InvalidArgument(
                    "Quote portal cannot resolve customer".to_string(),
                ))
            }
            common_grpc::middleware::server::auth::ResourceAccess::DashboardPortal(_) => {
                Err(PortalSharedApiError::InvalidArgument(
                    "Dashboard portal cannot resolve customer".to_string(),
                ))
            }
        }
    }

    /// Like [`Self::resolve_customer`], but a checkout token only sets up payment methods
    /// while its checkout can still be paid, and a bounded number of times (card testing).
    async fn resolve_customer_for_payment_setup(
        &self,
        resource: AuthorizedAsPortalUser,
        counts_as_setup_intent: bool,
    ) -> Result<CustomerId, Status> {
        let common_grpc::middleware::server::auth::ResourceAccess::CheckoutSession(session_id) =
            resource.resource_access
        else {
            return Ok(self.resolve_customer(resource).await?);
        };
        let session = self
            .store
            .get_checkout_session(resource.tenant_id, session_id)
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;
        if !session.can_complete() {
            return Err(Status::failed_precondition("This checkout is closed"));
        }
        if counts_as_setup_intent
            && !self
                .checkout_setup_limiter
                .check(session_id.as_base62())
                .await
                .is_allowed()
        {
            return Err(Status::resource_exhausted(
                "Too many payment method attempts for this checkout",
            ));
        }
        Ok(session.customer_id)
    }
}

#[tonic::async_trait]
impl PortalSharedService for PortalSharedServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn update_customer(
        &self,
        request: Request<UpdateCustomerRequest>,
    ) -> Result<Response<UpdateCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = self.resolve_customer(request.portal_resource()?).await?;

        let customer =
            request
                .into_inner()
                .customer
                .ok_or(PortalSharedApiError::MissingArgument(
                    "customer payload missing".to_string(),
                ))?;

        let billing_address = customer
            .billing_address
            .map(DomainAddressWrapper::try_from)
            .transpose()?
            .map(|v| v.0);
        let shipping_address = customer
            .shipping_address
            .map(DomainShippingAddressWrapper::try_from)
            .transpose()?
            .map(|v| v.0);

        let customer = self
            .store
            .patch_customer(
                Actor::Customer { id: customer_id },
                tenant_id,
                CustomerPatch {
                    id: customer_id,
                    name: customer.name.clone(),
                    legal_number: customer
                        .legal_number
                        .clone()
                        .map(|v| if v.is_empty() { None } else { Some(v) }),
                    alias: None,
                    billing_email: customer.billing_email.clone(),
                    invoicing_emails: None,
                    phone: customer.phone.clone(),
                    invoicing_entity_id: None,
                    currency: None,
                    billing_address,
                    shipping_address,
                    vat_number: customer
                        .vat_number
                        .map(|v| if v.is_empty() { None } else { Some(v) }),
                    custom_taxes: None,
                    current_payment_method_id: None,
                    tax_status: None,
                    exemption_reason: None,
                    connected_account_id: None,
                    custom_properties: None,
                    preferred_locales: None,
                    customer_type: None,
                    first_name: customer.first_name,
                    last_name: customer.last_name,
                    buyer_reference: None,
                },
            )
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(UpdateCustomerResponse {
            customer: customer
                .map(ServerCustomerWrapper::try_from)
                .transpose()
                .map_err(Into::<PortalSharedApiError>::into)?
                .map(|v| v.0),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn setup_intent(
        &self,
        request: Request<SetupIntentRequest>,
    ) -> Result<Response<SetupIntentResponse>, Status> {
        let tenant = request.tenant()?;
        let portal_resource = request.portal_resource()?;

        let inner = request.into_inner();

        // Only matters when the connection serves both rails (Mollie card + SEPA).
        let preferred_type = match inner.connection_type() {
            meteroid_grpc::meteroid::portal::shared::v1::ConnectionTypeEnum::Card => {
                meteroid_store::domain::ConnectionTypeEnum::Card
            }
            meteroid_grpc::meteroid::portal::shared::v1::ConnectionTypeEnum::DirectDebit => {
                meteroid_store::domain::ConnectionTypeEnum::DirectDebit
            }
        };
        let customer_connection_id = CustomerConnectionId::from_proto_opt(inner.connection_id)?
            // TODO: if connection_id is not provided, we could resolve the connector from the portal resource and create a new connection
            .ok_or(PortalSharedApiError::MissingArgument(
                "connection_id is required".to_string(),
            ))?;

        let customer_id = self
            .resolve_customer_for_payment_setup(portal_resource, true)
            .await?;
        let connection = self
            .store
            .get_connection_by_id(&tenant, &customer_connection_id)
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        if connection.customer_id != customer_id {
            return Err(PortalSharedApiError::InvalidArgument(
                "Connection does not belong to the resolved customer".to_string(),
            )
            .into());
        };

        // Optional: when the setup is paying a specific invoice, the invoice id
        // is stored in the mandate metadata so hosted-redirect providers can
        // charge the invoice once the mandate exists.
        let invoice_id = InvoiceId::from_proto_opt(inner.invoice_id)?;

        // The invoice id is client-supplied and is later charged off-session by
        // the `billing_requests.fulfilled` webhook, so it MUST be verified to
        // belong to the resolved customer here — otherwise customer A could set
        // up their mandate to pay (and mark Paid) customer B's invoice in the
        // same tenant. The webhook has no auth context to re-check this.
        if let Some(invoice_id) = invoice_id {
            let invoice = self
                .store
                .get_invoice_by_id(tenant, invoice_id)
                .await
                .map_err(Into::<PortalSharedApiError>::into)?;
            if invoice.customer_id != customer_id {
                return Err(PortalSharedApiError::InvalidArgument(
                    "Invoice does not belong to the resolved customer".to_string(),
                )
                .into());
            }
        }

        // For hosted-redirect providers (GoCardless) the frontend supplies
        // its desired post-redirect URL; embedded providers ignore it. We
        // pass it through verbatim — the adapter decides whether to use it.
        let intent = self
            .services
            .create_setup_intent(
                &tenant,
                &customer_connection_id,
                invoice_id,
                Some(preferred_type),
                inner.return_url,
                inner.descriptor_only.unwrap_or(false),
                inner.for_payment.unwrap_or(false),
            )
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        let provider = connector_provider_to_server(&intent.provider).ok_or_else(|| {
            PortalSharedApiError::InvalidArgument(
                "This provider cannot be used for setup intents".to_string(),
            )
        })?;

        Ok(Response::new(SetupIntentResponse {
            setup_intent: Some(SetupIntent {
                intent_id: intent.intent_id,
                intent_secret: intent.client_secret,
                provider_public_key: intent.public_key.expose_secret().to_string(),
                provider: provider as i32,
                connection_id: intent.connection_id.as_proto(),
                capabilities: meteroid_store::adapters::payment::payment_capabilities(
                    &intent.provider,
                )
                .map(crate::connectors::mapping::connectors::capabilities_to_server),
            }),
        }))
    }

    /// We want to process payment ASAP, without waiting for the webhook event, so this is a frontend-initiated action when stripe sdk confirm payment method.
    /// We will complete the details when the webhook event is received (if not already received)
    #[tracing::instrument(skip_all)]
    async fn add_payment_method(
        &self,
        request: Request<AddPaymentMethodRequest>,
    ) -> Result<Response<AddPaymentMethodResponse>, Status> {
        let tenant = request.tenant()?;
        let portal_resource = request.portal_resource()?;

        let customer_id = self
            .resolve_customer_for_payment_setup(portal_resource, false)
            .await?;

        let inner = request.into_inner();

        let connection_id = CustomerConnectionId::from_proto(inner.connection_id)?;
        let external_payment_method_id = inner.external_payment_method_id;
        let for_payment = inner.for_payment;

        let connection = self
            .store
            .get_connection_by_id(&tenant, &connection_id)
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        if connection.customer_id != customer_id {
            return Err(PortalSharedApiError::InvalidArgument(
                "Connection does not belong to the resolved customer".to_string(),
            )
            .into());
        }

        // Fetch payment method details from provider to get the actual type
        let connector = self
            .store
            .get_connector_with_data(connection.connector_id, tenant)
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        let connector_impl = initialize_payment_connector(&connector)
            .change_context(StoreError::PaymentProviderError)
            .map_err(Into::<PortalSharedApiError>::into)?;

        let method = connector_impl
            .fetch_payment_method(
                &external_payment_method_id,
                &connection.external_customer_id,
            )
            .await
            .change_context(StoreError::PaymentProviderError)
            .map_err(Into::<PortalSharedApiError>::into)?;

        let payment_method = self
            .store
            // not an upsert as the WH event is more precise
            .insert_payment_method_if_not_exist(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id: tenant,
                customer_id: connection.customer_id,
                connection_id,
                external_payment_method_id,
                payment_method_type: method.payment_method_type,
                account_number_hint: method.account_number_hint,
                card_brand: method.card_brand,
                card_last4: method.card_last4,
                card_exp_month: method.card_exp_month,
                card_exp_year: method.card_exp_year,
                fingerprint: method.fingerprint,
                saved_for_payment: for_payment,
            })
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        // Added in the portal's settings it is the new default; saved to pay now it waits for that
        // payment to settle, so a declined card never becomes the default.
        if !for_payment {
            self.services
                .adopt_default_payment_method(
                    Actor::Customer { id: customer_id },
                    tenant,
                    connection.customer_id,
                    payment_method.id,
                    meteroid_store::services::DefaultMethodAdoption::Replace,
                )
                .await
                .map_err(Into::<PortalSharedApiError>::into)?;
        }

        Ok(Response::new(AddPaymentMethodResponse {
            payment_method: Some(
                crate::customers::mapping::customer_payment_method::domain_to_server(
                    payment_method,
                ),
            ),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_default_payment_method(
        &self,
        request: Request<SetDefaultPaymentMethodRequest>,
    ) -> Result<Response<SetDefaultPaymentMethodResponse>, Status> {
        let tenant = request.tenant()?;
        let portal_resource = request.portal_resource()?;
        let customer_id = self.resolve_customer(portal_resource).await?;

        let inner = request.into_inner();
        let payment_method_id = CustomerPaymentMethodId::from_proto(inner.payment_method_id)?;

        // The payment method must belong to the authenticated customer.
        let pm = self
            .store
            .get_payment_method_by_id(&tenant, &payment_method_id)
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        if pm.customer_id != customer_id {
            return Err(PortalSharedApiError::InvalidArgument(
                "Payment method does not belong to the resolved customer".to_string(),
            )
            .into());
        }

        let customer_patch = CustomerPatch {
            id: customer_id,
            name: None,
            legal_number: None,
            buyer_reference: None,
            alias: None,
            billing_email: None,
            phone: None,
            currency: None,
            billing_address: None,
            shipping_address: None,
            invoicing_entity_id: None,
            vat_number: None,
            current_payment_method_id: Some(Some(payment_method_id)),
            invoicing_emails: None,
            tax_status: None,
            exemption_reason: None,
            custom_taxes: None,
            connected_account_id: None,
            custom_properties: None,
            preferred_locales: None,
            customer_type: None,
            first_name: None,
            last_name: None,
        };

        let updated = self
            .store
            .patch_customer(Actor::Customer { id: customer_id }, tenant, customer_patch)
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        let customer = match updated {
            Some(c) => c,
            None => self
                .store
                .find_customer_by_id(customer_id, tenant)
                .await
                .map_err(Into::<PortalSharedApiError>::into)?,
        };

        let customer_proto = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalSharedApiError>::into)?;

        Ok(Response::new(SetDefaultPaymentMethodResponse {
            customer: Some(customer_proto),
        }))
    }
}
