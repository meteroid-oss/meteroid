use crate::api::customers::mapping::customer::ServerCustomerWrapper;
use crate::api::invoices::mapping;
use crate::api::portal::invoice::PortalInvoiceServiceComponents;
use crate::api::portal::invoice::error::PortalInvoiceApiError;
use crate::services::storage::Prefix;
use common_domain::ids::{CustomerPaymentMethodId, InvoiceId};

use common_grpc::middleware::server::auth::{RequestExt, ResourceAccess};
use meteroid_grpc::meteroid::portal::invoice::v1::portal_invoice_service_server::PortalInvoiceService;
use meteroid_grpc::meteroid::portal::invoice::v1::*;
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;

use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{
    InvoiceInterface, OrganizationsInterface, SubscriptionInterface,
};
use std::time::Duration;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl PortalInvoiceService for PortalInvoiceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_invoice_payment(
        &self,
        request: Request<GetInvoiceForPaymentRequest>,
    ) -> Result<Response<GetInvoiceForPaymentResponse>, Status> {
        let tenant = request.tenant()?;

        let (invoice_id, customer_id) = match request.portal_resource()?.resource_access {
            ResourceAccess::InvoicePortal(id) => Ok((id, None)),
            ResourceAccess::CustomerPortal(id) => {
                let invoice_id = InvoiceId::from_proto_opt(request.into_inner().invoice_id)?
                    .ok_or(Status::invalid_argument("Missing invoice ID in request"))?;
                Ok((invoice_id, Some(id)))
            }
            _ => Err(Status::invalid_argument(
                "Resource is not an invoice or customer portal.",
            )),
        }?;

        let transactions = self
            .store
            .list_payment_tx_by_invoice_id(tenant, invoice_id)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?
            .into_iter()
            .map(mapping::transactions::domain_with_method_to_server)
            .collect::<Vec<_>>();

        let invoice = self
            .store
            .get_detailed_invoice_by_id(tenant, invoice_id)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        if let Some(cid) = customer_id
            && invoice.invoice.customer_id != cid
        {
            return Err(Status::permission_denied(
                "Invoice does not belong to the specified customer.",
            ));
        }

        let customer = self
            .store
            .find_customer_by_id(invoice.invoice.customer_id, tenant)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant, Some(customer.invoicing_entity_id))
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        // Resolve payment method availability based on subscription payment configuration
        // This uses the hybrid approach:
        // - For subscriptions with Inherit config: uses CURRENT invoicing entity providers
        // - For subscriptions with Override config: uses specifically enabled methods
        // - For standalone invoices: creates customer connections if needed
        let resolved = if let Some(subscription_id) = invoice.invoice.subscription_id {
            let subscription = self
                .store
                .get_subscription(tenant, subscription_id)
                .await
                .map_err(Into::<PortalInvoiceApiError>::into)?;

            // Resolve payment methods at runtime based on subscription's config
            self.services
                .resolve_subscription_payment_methods(
                    tenant,
                    subscription.payment_methods_config.as_ref(),
                    &customer,
                )
                .await
                .map_err(Into::<PortalInvoiceApiError>::into)?
        } else {
            // Standalone invoice - use invoicing entity's providers (equivalent to Inherit)
            self.services
                .resolve_subscription_payment_methods(tenant, None, &customer)
                .await
                .map_err(Into::<PortalInvoiceApiError>::into)?
        };

        let card_connection_id = resolved.card_connection_id;
        let direct_debit_connection_id = resolved.direct_debit_connection_id;
        let bank_transfer_enabled = resolved.bank_transfer_enabled;
        let bank_account_id_override = resolved.bank_account_id;

        // Fetch customer payment methods and filter to only those usable with resolved connections
        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        let payment_methods = resolved
            .filter_payment_methods(customer_methods)
            .into_iter()
            .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let mut invoice =
            crate::api::invoices::mapping::invoices::domain_invoice_with_transactions_to_server(
                invoice.invoice,
                invoice.transactions,
                self.jwt_secret.clone(),
            )
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        invoice.transactions = transactions;

        let customer = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        log::info!(
            "logo_attachment_id: {:?}",
            invoicing_entity.logo_attachment_id
        );

        let logo_url = if let Some(logo_attachment_id) = invoicing_entity.logo_attachment_id {
            self.object_store
                .get_url(
                    logo_attachment_id,
                    Prefix::ImageLogo,
                    Duration::from_secs(7 * 86400),
                )
                .await
                .map_err(Into::<PortalInvoiceApiError>::into)?
        } else {
            None
        };

        log::info!("logo_url: {:?}", logo_url);

        // Only show bank account if bank transfer is enabled in the resolved config
        // For External subscriptions, bank_transfer_enabled is false so no fallback occurs
        let bank_account = if bank_transfer_enabled
            && card_connection_id.is_none()
            && direct_debit_connection_id.is_none()
        {
            // Prefer subscription's bank account (set by payment strategy),
            // otherwise use invoicing entity's default
            let bank_account_id_to_use =
                bank_account_id_override.or(invoicing_entity.bank_account_id);
            if let Some(bank_account_id) = bank_account_id_to_use {
                self.store
                    .get_bank_account_by_id(bank_account_id, tenant)
                    .await
                    .ok()
                    .map(crate::api::bankaccounts::mapping::bank_accounts::domain_to_proto)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Response::new(GetInvoiceForPaymentResponse {
            invoice: Some(InvoiceForPayment {
                invoice: Some(invoice),
                customer: Some(customer),
                payment_methods,
                logo_url,
                trade_name: organization.trade_name,
                card_connection_id: card_connection_id.map(|t| t.as_proto()),
                direct_debit_connection_id: direct_debit_connection_id.map(|t| t.as_proto()),
                bank_account,
                footer_legal: invoicing_entity.invoice_footer_legal,
                legal_number: invoicing_entity.vat_number,
                footer_info: invoicing_entity.invoice_footer_info,
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn confirm_invoice_payment(
        &self,
        request: Request<ConfirmInvoicePaymentRequest>,
    ) -> Result<Response<ConfirmInvoicePaymentResponse>, Status> {
        let tenant = request.tenant()?;

        let resource = request.portal_resource()?;

        let inner = request.into_inner();

        let (invoice_id, customer_id) = match resource.resource_access {
            ResourceAccess::InvoicePortal(id) => Ok((id, None)),
            ResourceAccess::CustomerPortal(id) => {
                let invoice_id = InvoiceId::from_proto_opt(inner.invoice_id)?
                    .ok_or(Status::invalid_argument("Missing invoice ID in request"))?;
                Ok((invoice_id, Some(id)))
            }
            _ => Err(Status::invalid_argument(
                "Resource is not an invoice or customer portal.",
            )),
        }?;

        let invoice = self
            .store
            .get_invoice_by_id(tenant, invoice_id)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        if let Some(customer_id) = customer_id
            && invoice.customer_id != customer_id
        {
            return Err(Status::permission_denied(
                "Invoice does not belong to the specified customer.",
            ));
        }

        let payment_method_id = CustomerPaymentMethodId::from_proto(inner.payment_method_id)?;

        if invoice.currency != inner.displayed_currency {
            return Err(Status::invalid_argument(
                "Displayed currency does not match invoice currency.",
            ));
        }

        if invoice.amount_due != inner.displayed_amount as i64 {
            return Err(Status::invalid_argument(
                "Displayed amount does not match invoice amount due.",
            ));
        }

        let transaction = self
            .services
            .complete_invoice_payment(tenant, invoice_id, payment_method_id)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        Ok(Response::new(ConfirmInvoicePaymentResponse {
            transaction: Some(
                crate::api::invoices::mapping::transactions::domain_to_server(transaction),
            ),
        }))
    }
}
