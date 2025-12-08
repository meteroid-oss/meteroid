use crate::api::connectors::mapping::connectors::connector_provider_to_server;
use crate::api::customers::error::CustomerApiError;
use crate::api::customers::mapping::customer::{
    DomainAddressWrapper, DomainShippingAddressWrapper, ServerCustomerWrapper,
};
use crate::api::portal::shared::PortalSharedServiceComponents;
use crate::api::portal::shared::error::PortalSharedApiError;
use common_domain::ids::{BaseId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId};
use common_grpc::middleware::server::auth::{AuthorizedAsPortalUser, RequestExt};
use error_stack::ResultExt;
use meteroid_grpc::meteroid::portal::shared::v1::portal_shared_service_server::PortalSharedService;
use meteroid_grpc::meteroid::portal::shared::v1::*;
use meteroid_store::adapters::payment_service_providers::initialize_payment_provider;
use meteroid_store::domain::{CustomerPatch, CustomerPaymentMethodNew};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::{InvoiceInterface, SubscriptionInterface};
use secrecy::ExposeSecret;
use tonic::{Request, Response, Status};

impl PortalSharedServiceComponents {
    async fn resolve_customer(
        &self,
        resource: AuthorizedAsPortalUser,
    ) -> Result<CustomerId, PortalSharedApiError> {
        match resource.resource_access {
            common_grpc::middleware::server::auth::ResourceAccess::SubscriptionCheckout(
                subscription_id,
            ) => {
                let subscription = self
                    .store
                    .get_subscription(resource.tenant_id, subscription_id)
                    .await
                    .map_err(Into::<PortalSharedApiError>::into)?;

                Ok(subscription.customer_id)
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
            _ => Err(PortalSharedApiError::InvalidArgument(
                "Invalid portal resource for customer resolution".to_string(),
            )),
        }
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
                customer_id.as_uuid(), // TODO Customer as actor, we need to change the actor system
                tenant_id,
                CustomerPatch {
                    id: customer_id,
                    name: customer.name.clone(),
                    alias: None,
                    billing_email: customer.billing_email.clone(),
                    invoicing_emails: None,
                    phone: customer.phone.clone(),
                    balance_value_cents: None,
                    invoicing_entity_id: None,
                    currency: None,
                    billing_address,
                    shipping_address,
                    vat_number: customer
                        .vat_number
                        .map(|v| if v.is_empty() { None } else { Some(v) }),
                    custom_taxes: None,
                    bank_account_id: None,
                    current_payment_method_id: None,
                    is_tax_exempt: None,
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
        let customer_connection_id = CustomerConnectionId::from_proto_opt(inner.connection_id)?
            // TODO: if connection_id is not provided, we could resolve the connector from the portal resource and create a new connection
            .ok_or(PortalSharedApiError::MissingArgument(
                "connection_id is required".to_string(),
            ))?;

        let customer_id = self.resolve_customer(portal_resource).await?;
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

        let intent = self
            .services
            .create_setup_intent(&tenant, &customer_connection_id) // connection_type
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        Ok(Response::new(SetupIntentResponse {
            setup_intent: Some(SetupIntent {
                intent_id: intent.intent_id,
                intent_secret: intent.client_secret,
                provider_public_key: intent.public_key.expose_secret().to_string(),
                provider: connector_provider_to_server(&intent.provider) as i32,
                connection_id: intent.connection_id.as_proto(),
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

        let customer_id = self.resolve_customer(portal_resource).await?;

        let inner = request.into_inner();

        let connection_id = CustomerConnectionId::from_proto(inner.connection_id)?;
        let external_payment_method_id = inner.external_payment_method_id;

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

        let provider = initialize_payment_provider(&connector)
            .change_context(StoreError::PaymentProviderError)
            .map_err(Into::<PortalSharedApiError>::into)?;

        let method = provider
            .get_payment_method_from_provider(
                &connector,
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
            })
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        // Set as default payment method for the customer
        let customer_patch = CustomerPatch {
            id: connection.customer_id,
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
            bank_account_id: None,
            current_payment_method_id: Some(Some(payment_method.id)),
            invoicing_emails: None,
            is_tax_exempt: None,
            custom_taxes: None,
        };

        self.store
            .patch_customer(customer_id.as_uuid(), tenant, customer_patch)
            .await
            .map_err(Into::<PortalSharedApiError>::into)?;

        Ok(Response::new(AddPaymentMethodResponse {
            payment_method: Some(
                crate::api::customers::mapping::customer_payment_method::domain_to_server(
                    payment_method,
                ),
            ),
        }))
    }
}
