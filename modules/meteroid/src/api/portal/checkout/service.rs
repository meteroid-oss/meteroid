use crate::api::connectors::mapping::connectors::connector_provider_to_server;
use crate::api::customers::error::CustomerApiError;
use crate::api::customers::mapping::customer::{
    DomainAddressWrapper, DomainShippingAddressWrapper, ServerCustomerWrapper,
};
use crate::api::portal::checkout::PortalCheckoutServiceComponents;
use crate::api::portal::checkout::error::PortalCheckoutApiError;
use crate::services::storage::Prefix;
use common_domain::ids::{
    BankAccountId, BaseId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
    InvoicingEntityId,
};
use common_grpc::middleware::server::auth::RequestExt;
use common_utils::decimals::ToSubunit;
use common_utils::integers::ToNonNegativeU64;
use error_stack::ResultExt;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_server::PortalCheckoutService;
use meteroid_grpc::meteroid::portal::checkout::v1::{
    AddPaymentMethodRequest, AddPaymentMethodResponse, AppliedCoupon, Checkout,
    ConfirmCheckoutRequest, ConfirmCheckoutResponse, ConfirmSlotUpgradeCheckoutRequest,
    ConfirmSlotUpgradeCheckoutResponse, GetSlotUpgradeCheckoutRequest,
    GetSlotUpgradeCheckoutResponse, GetSubscriptionCheckoutRequest,
    GetSubscriptionCheckoutResponse, SetupIntent, SetupIntentRequest, SetupIntentResponse,
    SlotUpgradeCheckout, TaxBreakdownItem, UpdateCustomerRequest, UpdateCustomerResponse,
};
use meteroid_store::constants::Currencies;
use meteroid_store::domain::Period;
use meteroid_store::domain::{
    CustomerPatch, CustomerPaymentMethodNew, PaymentMethodTypeEnum, SubscriptionFeeInterface,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::customer_connection::CustomerConnectionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::{OrganizationsInterface, SubscriptionInterface};
use meteroid_store::utils::periods::calculate_proration_factor;
use rust_decimal::prelude::FromPrimitive;
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

        let subscription = self
            .store
            .get_subscription_details(tenant, subscription)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let invoice_content = self
            .services
            .compute_invoice(&subscription.subscription.start_date, &subscription, None)
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

        let subscription =
            crate::api::subscriptions::mapping::subscriptions::details_domain_to_proto(
                subscription,
            )?;

        let customer = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let logo_url = if let Some(logo_attachment_id) = invoicing_entity.logo_attachment_id {
            self.object_store
                .get_url(
                    logo_attachment_id,
                    Prefix::ImageLogo,
                    Duration::from_secs(7 * 86400),
                )
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?
        } else {
            None
        };

        let invoice_lines = crate::api::invoices::mapping::invoices::domain_invoice_lines_to_server(
            invoice_content.invoice_lines,
        );

        // Map tax breakdown
        let tax_breakdown = invoice_content
            .tax_breakdown
            .into_iter()
            .map(|item| TaxBreakdownItem {
                name: item.name,
                rate: item.tax_rate.to_string(),
                amount: item.tax_amount,
            })
            .collect();

        // Map applied coupons
        let applied_coupons = invoice_content
            .applied_coupons
            .clone()
            .into_iter()
            .map(|coupon| AppliedCoupon {
                coupon_code: coupon.code,
                coupon_name: coupon.name,
                amount: coupon.value.to_non_negative_u64(),
            })
            .collect();

        // Calculate total coupon amount
        let coupon_amount: i64 = invoice_content
            .applied_coupons
            .iter()
            .map(|c| c.value)
            .sum();

        Ok(Response::new(GetSubscriptionCheckoutResponse {
            checkout: Some(Checkout {
                subscription: Some(subscription),
                customer: Some(customer),
                invoice_lines,
                logo_url,
                trade_name: organization.trade_name,
                payment_methods,
                total_amount: invoice_content.total.to_non_negative_u64(),
                subtotal_amount: invoice_content.subtotal.to_non_negative_u64(),
                tax_amount: invoice_content.tax_amount.to_non_negative_u64(),
                discount_amount: invoice_content.discount.to_non_negative_u64(),
                coupon_amount: coupon_amount.to_non_negative_u64(),
                tax_breakdown,
                applied_coupons,
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
                    custom_taxes: None,
                    bank_account_id: Some(BankAccountId::from_proto_opt(customer.bank_account_id)?),
                    is_tax_exempt: customer.is_tax_exempt,
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
            .services
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
            .services
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

    #[tracing::instrument(skip_all)]
    async fn get_slot_upgrade_checkout(
        &self,
        request: Request<GetSlotUpgradeCheckoutRequest>,
    ) -> Result<Response<GetSlotUpgradeCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let subscription = request.portal_resource()?.subscription()?;

        let inner = request.into_inner();

        let price_component_id =
            common_domain::ids::PriceComponentId::from_proto(inner.price_component_id)?;

        let subscription_details = self
            .store
            .get_subscription_details(tenant, subscription)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let slot_component = subscription_details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .ok_or_else(|| {
                tonic::Status::not_found(format!(
                    "Price component {} not found",
                    price_component_id
                ))
            })?;

        let unit_name = match slot_component.fee_ref() {
            meteroid_store::domain::SubscriptionFee::Slot { unit, .. } => unit.clone(),
            _ => {
                return Err(tonic::Status::invalid_argument(
                    "Price component is not a slot component",
                ));
            }
        };

        let result = self
            .services
            .update_subscription_slots(
                tenant,
                subscription,
                price_component_id,
                inner.delta,
                meteroid_store::domain::SlotUpgradeBillingMode::OnCheckout,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let customer = self
            .store
            .find_customer_by_id(subscription_details.subscription.customer_id, tenant)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
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

        let currency_code = subscription_details.subscription.currency.clone();
        let currency = Currencies::resolve_currency(&currency_code).ok_or_else(|| {
            tonic::Status::internal(format!("Currency {} not found", currency_code))
        })?;

        let subscription_proto =
            crate::api::subscriptions::mapping::subscriptions::details_domain_to_proto(
                subscription_details,
            )?;

        let customer_proto = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let logo_url = if let Some(logo_attachment_id) = invoicing_entity.logo_attachment_id {
            self.object_store
                .get_url(
                    logo_attachment_id,
                    Prefix::ImageLogo,
                    Duration::from_secs(3600 * 24),
                )
                .await
                .map_err(Into::<PortalCheckoutApiError>::into)?
        } else {
            None
        };

        let prorated_amount_subunits = result
            .prorated_amount
            .and_then(|amount| amount.to_subunit_opt(currency.precision))
            .unwrap_or(0);

        if prorated_amount_subunits <= 0 {
            return Err(tonic::Status::invalid_argument(
                "Prorated change is 0 or negative",
            ));
        }

        let prorated_amount_subunits = prorated_amount_subunits.to_non_negative_u64();

        Ok(Response::new(GetSlotUpgradeCheckoutResponse {
            checkout: Some(SlotUpgradeCheckout {
                subscription: Some(subscription_proto),
                customer: Some(customer_proto),
                payment_methods,
                price_component_id: price_component_id.as_proto(),
                unit_name,
                current_slot_count: (result.new_slot_count - inner.delta) as u32,
                new_slot_count: result.new_slot_count as u32,
                delta: inner.delta,
                prorated_amount: prorated_amount_subunits,
                currency: currency_code,
                logo_url,
                trade_name: organization.trade_name,
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn confirm_slot_upgrade_checkout(
        &self,
        request: Request<ConfirmSlotUpgradeCheckoutRequest>,
    ) -> Result<Response<ConfirmSlotUpgradeCheckoutResponse>, Status> {
        let tenant = request.tenant()?;
        let subscription = request.portal_resource()?.subscription()?;

        let inner = request.into_inner();

        let price_component_id =
            common_domain::ids::PriceComponentId::from_proto(inner.price_component_id)?;
        let payment_method_id = CustomerPaymentMethodId::from_proto(inner.payment_method_id)?;

        let subscription_details = self
            .store
            .get_subscription_details(tenant, subscription)
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        let slot_component = subscription_details
            .price_components
            .iter()
            .find(|c| c.price_component_id() == Some(price_component_id))
            .ok_or_else(|| tonic::Status::not_found("Price component not found"))?;

        let (unit_rate, _unit) = match slot_component.fee_ref() {
            meteroid_store::domain::SubscriptionFee::Slot {
                unit_rate, unit, ..
            } => (unit_rate, unit),
            _ => return Err(tonic::Status::invalid_argument("Not a slot component")),
        };

        let now = chrono::Utc::now().date_naive();
        let period_end = subscription_details
            .subscription
            .current_period_end
            .ok_or_else(|| tonic::Status::invalid_argument("No current_period_end"))?;

        let period = Period {
            start: now,
            end: period_end,
        };
        let proration_factor = calculate_proration_factor(&period);
        let base_amount = rust_decimal::Decimal::from(inner.delta) * unit_rate;
        let prorated = if let Some(factor) = proration_factor {
            base_amount
                * rust_decimal::Decimal::from_f64(factor).unwrap_or(rust_decimal::Decimal::ONE) // TODO
        } else {
            base_amount
        };
        let expected_amount_subunits = prorated
            .max(rust_decimal::Decimal::ZERO)
            .to_subunit_opt(2)
            .unwrap_or(0);

        // Validate with tolerance
        let displayed_amount = inner.displayed_amount;
        let diff = (expected_amount_subunits as i64 - displayed_amount as i64).abs();
        if diff > 1 {
            return Err(tonic::Status::invalid_argument(format!(
                "Amount mismatch: displayed {} but expected {} (currency: {})",
                displayed_amount, expected_amount_subunits, inner.displayed_currency
            )));
        }

        if inner.displayed_currency != subscription_details.subscription.currency {
            return Err(tonic::Status::invalid_argument(format!(
                "Currency mismatch: displayed {} but expected {}",
                inner.displayed_currency, subscription_details.subscription.currency
            )));
        }

        let (transaction, new_slot_count) = self
            .services
            .complete_slot_upgrade_checkout(
                tenant,
                subscription,
                price_component_id,
                inner.delta,
                payment_method_id,
                None,
            )
            .await
            .map_err(Into::<PortalCheckoutApiError>::into)?;

        Ok(Response::new(ConfirmSlotUpgradeCheckoutResponse {
            transaction: Some(
                crate::api::invoices::mapping::transactions::domain_to_server(transaction),
            ),
            new_slot_count: new_slot_count as u32,
        }))
    }
}
