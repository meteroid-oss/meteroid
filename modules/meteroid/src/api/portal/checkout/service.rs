use crate::api::connectors::mapping::connectors::connector_provider_to_server;
use crate::api::customers::error::CustomerApiError;
use crate::api::customers::mapping::customer::{
    DomainAddressWrapper, DomainShippingAddressWrapper, ServerCustomerWrapper,
};
use crate::api::portal::checkout::PortalCheckoutServiceComponents;
use crate::api::portal::checkout::error::PortalCheckoutApiError;
use crate::services::storage::Prefix;
use crate::{api::utils::parse_uuid, parse_uuid};
use common_domain::ids::{
    BaseId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId, InvoicingEntityId,
};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::ResultExt;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_server::PortalCheckoutService;
use meteroid_grpc::meteroid::portal::checkout::v1::*;
use meteroid_store::compute::InvoiceLineInterface;
use meteroid_store::domain::{
    CustomerPatch, CustomerPaymentMethodNew, InvoiceTotals, InvoiceTotalsParams,
    PaymentMethodTypeEnum,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::billing::BillingService;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::{OrganizationsInterface, SubscriptionInterface};
use secrecy::ExposeSecret;
use std::time::Duration;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl PortalCheckoutService for PortalCheckoutServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_subscription_checkout(
        &self,
        request: Request<GetSubscriptionCheckoutRequest>,
    ) -> Result<Response<GetSubscriptionCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let subscription = request.portal_resource()?.subscription()?;

        // TODO single query

        let subscription = self
            .store
            .get_subscription_details(tenant, subscription)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let invoice_lines = self
            .store
            .compute_dated_invoice_lines(&subscription.subscription.start_date, &subscription)
            .await
            .change_context(StoreError::InvoiceComputationError)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(subscription.subscription.customer_id, tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant, Some(customer.invoicing_entity_id))
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;
        let payment_methods = customer_methods
            .into_iter()
            .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let totals = InvoiceTotals::from_params(InvoiceTotalsParams {
            line_items: &invoice_lines,
            total: 0, // no prepaid (TODO check)
            amount_due: 0,
            tax_rate: 0,
            customer_balance_cents: customer.balance_value_cents,
            subscription_applied_coupons: &vec![], // TODO
            invoice_currency: subscription.subscription.currency.as_str(),
        });

        let subscription =
            crate::api::subscriptions::mapping::subscriptions::details_domain_to_proto(
                subscription,
            )?;

        let customer = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let logo_url = if let Some(logo_attachment_id) = invoicing_entity.logo_attachment_id {
            let logo_uuid = parse_uuid!(logo_attachment_id)?;

            self.object_store
                .get_url(logo_uuid, Prefix::ImageLogo, Duration::from_secs(7 * 86400))
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?
        } else {
            None
        };

        let invoice_lines =
            crate::api::invoices::mapping::invoices::domain_invoice_lines_to_server(invoice_lines);

        Ok(Response::new(GetSubscriptionCheckoutResponse {
            checkout: Some(Checkout {
                subscription: Some(subscription),
                customer: Some(customer),
                invoice_lines,
                logo_url,
                trade_name: organization.trade_name,
                payment_methods,
                // TODO recurring_total => also check this is not prorated
                // amount_due
                total_amount: totals.total as u64,
                subtotal_amount: totals.subtotal as u64,
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_customer(
        &self,
        request: Request<UpdateCustomerRequest>,
    ) -> Result<Response<UpdateCustomerResponse>, Status> {
        let tenant_id = request.tenant()?;
        // TODO check subscription

        let customer =
            request
                .into_inner()
                .customer
                .ok_or(PortalCheckoutApiError::MissingArgument(
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

        let customer_id = CustomerId::from_proto(&customer.id)?;
        let customer = self
            .store
            .patch_customer(
                customer_id.as_uuid(), // TODO Customer as actor, we need to change the actor system
                tenant_id,
                CustomerPatch {
                    id: customer_id,
                    name: customer.name.clone(),
                    alias: customer.alias.clone(),
                    billing_email: customer.billing_email.clone(),
                    invoicing_emails: customer.invoicing_emails.map(|v| v.emails),
                    phone: customer.phone.clone(),
                    balance_value_cents: customer.balance_value_cents,
                    invoicing_entity_id: InvoicingEntityId::from_proto_opt(
                        customer.invoicing_entity_id,
                    )?,
                    currency: customer.currency.clone(),
                    billing_address,
                    shipping_address,
                    vat_number: customer
                        .vat_number
                        .map(|v| if v.is_empty() { None } else { Some(v) }),
                    custom_vat_rate: None,
                },
            )
            .await
            .map_err(Into::<CustomerApiError>::into)?;

        Ok(Response::new(UpdateCustomerResponse {
            customer: customer
                .map(ServerCustomerWrapper::try_from)
                .transpose()
                .map_err(Into::<PortalCheckoutApiError>::into)?
                .map(|v| v.0),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn setup_intent(
        &self,
        request: Request<SetupIntentRequest>,
    ) -> Result<Response<SetupIntentResponse>, Status> {
        let tenant = request.tenant()?;
        let subscription = request.portal_resource()?.subscription()?;

        let inner = request.into_inner();

        let subscription = self
            .store
            .get_subscription(tenant, subscription)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let customer_connection_id = CustomerConnectionId::from_proto(&inner.connection_id)?;

        // validate that customer_connection_id is either subscription.card_provider_id or subscription.direct_debit_provider_id
        let is_valid = match (
            &subscription.card_connection_id,
            &subscription.direct_debit_connection_id,
        ) {
            (Some(card_id), _) if *card_id == customer_connection_id => true,
            (_, Some(debit_id)) if *debit_id == customer_connection_id => true,
            _ => false,
        };

        if !is_valid {
            Err(PortalCheckoutApiError::InvalidArgument(
                "Connection is not valid for this subscription".to_string(),
            ))?;
        }

        let intent = self
            .store
            .create_setup_intent(&tenant, &customer_connection_id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        Ok(Response::new(SetupIntentResponse {
            setup_intent: Some(SetupIntent {
                intent_id: intent.intent_id,
                intent_secret: intent.client_secret,
                provider_public_key: intent.public_key.expose_secret().clone(),
                provider: connector_provider_to_server(&intent.provider) as i32,
                connection_id: intent.connection_id.as_proto(),
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn confirm_checkout(
        &self,
        request: Request<ConfirmCheckoutRequest>,
    ) -> Result<Response<ConfirmCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let subscription = request.portal_resource()?.subscription()?;

        let inner = request.into_inner();

        let payment_method_id = CustomerPaymentMethodId::from_proto(inner.payment_method_id)?;

        let transaction = self
            .store
            .complete_subscription_checkout(
                tenant,
                subscription,
                payment_method_id,
                inner.displayed_amount,
                inner.displayed_currency,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        Ok(Response::new(ConfirmCheckoutResponse {
            transaction: Some(
                crate::api::invoices::mapping::transactions::domain_to_server(transaction),
            ),
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
        let subscription = request.portal_resource()?.subscription()?;

        let inner = request.into_inner();

        let connection_id = CustomerConnectionId::from_proto(inner.connection_id)?;
        let external_payment_method_id = inner.external_payment_method_id;

        let subscription = self
            .store
            .get_subscription(tenant, subscription)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let connection = self
            .store
            .get_connection_by_id(&tenant, &connection_id)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        if subscription.customer_id != connection.customer_id {
            return Err(PortalCheckoutApiError::InvalidArgument(
                "Subscription customer is not attached to this connection".to_string(),
            )
            .into());
        }

        let payment_method = self
            .store
            .insert_payment_method_if_not_exist(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id: tenant,
                customer_id: connection.customer_id,
                connection_id,
                external_payment_method_id,
                payment_method_type: PaymentMethodTypeEnum::Card, // TODO
                account_number_hint: None,
                card_brand: None,
                card_last4: None,
                card_exp_month: None,
                card_exp_year: None,
            })
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        Ok(Response::new(AddPaymentMethodResponse {
            payment_method: Some(
                crate::api::customers::mapping::customer_payment_method::domain_to_server(
                    payment_method,
                ),
            ),
        }))
    }
}
