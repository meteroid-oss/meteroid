use crate::api::customers::mapping::customer::ServerCustomerWrapper;
use crate::api::portal::invoice::PortalInvoiceServiceComponents;
use crate::api::portal::invoice::error::PortalInvoiceApiError;
use crate::api::shared::conversions::FromProtoOpt;
use crate::services::storage::Prefix;
use common_domain::ids::{BaseId, CustomerPaymentMethodId, InvoiceId};
use common_grpc::middleware::server::auth::{RequestExt, ResourceAccess};
use meteroid_grpc::meteroid::portal::invoice::v1::portal_invoice_service_server::PortalInvoiceService;
use meteroid_grpc::meteroid::portal::invoice::v1::*;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::{InvoiceInterface, OrganizationsInterface};
use std::time::Duration;
use tonic::{Request, Response, Status};
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;


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
                let invoice_id = InvoiceId::from_proto_opt( request.into_inner().invoice_id)?
                    .ok_or(Status::invalid_argument("Missing invoice ID in request"))?;
                Ok((invoice_id, Some(id)))
            },
            _ => Err(Status::invalid_argument(
                "Resource is not an invoice or customer portal.",
            )),
        }?;

        let invoice = self
            .store
            .get_detailed_invoice_by_id(tenant, invoice_id)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        if let Some(cid) = customer_id
            && invoice.invoice.customer_id != cid {
                return Err(Status::permission_denied("Invoice does not belong to the specified customer."));
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

        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        let payment_methods = customer_methods
            .into_iter()
            .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let organization = self
            .store
            .get_organization_by_tenant_id(&tenant)
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        // Get or create customer connections for payment providers
        let (card_connection_id, direct_debit_connection_id) = self
            .services
            .get_or_create_customer_connections(
                tenant,
                customer.id,
                customer.invoicing_entity_id,
            )
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        let invoice = crate::api::invoices::mapping::invoices::domain_invoice_with_transactions_to_server(
            invoice.invoice,
            invoice.transactions,
            self.jwt_secret.clone(),
        ).map_err(Into::<PortalInvoiceApiError>::into)?;

        let customer = ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalInvoiceApiError>::into)?;

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

        // Get bank account if configured
        let bank_account = if let Some(bank_account_id) = invoicing_entity.bank_account_id {
            self.store
                .get_bank_account_by_id(bank_account_id, tenant)
                .await
                .ok()
                .map(crate::api::bankaccounts::mapping::bank_accounts::domain_to_proto)
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
                let invoice_id = InvoiceId::from_proto_opt( inner.invoice_id)?
                    .ok_or(Status::invalid_argument("Missing invoice ID in request"))?;
                Ok((invoice_id, Some(id)))
            },
            _ => Err(Status::invalid_argument(
                "Resource is not an invoice or customer portal.",
            )),
        }?;

        if let Some(customer_id) = customer_id {
            let invoice = self
                .store
                .get_invoice_by_id(tenant, invoice_id)
                .await
                .map_err(Into::<PortalInvoiceApiError>::into)?;

            if invoice.customer_id != customer_id {
                return Err(Status::permission_denied("Invoice does not belong to the specified customer."));
            }
        }


        let payment_method_id = CustomerPaymentMethodId::from_proto(inner.payment_method_id)?;

        let transaction = self
            .services
            .complete_invoice_payment(
                tenant,
                invoice_id,
                payment_method_id,
                inner.displayed_amount,
                inner.displayed_currency,
            )
            .await
            .map_err(Into::<PortalInvoiceApiError>::into)?;

        Ok(Response::new(ConfirmInvoicePaymentResponse {
            transaction: Some(
                crate::api::invoices::mapping::transactions::domain_to_server(transaction),
            ),
        }))
    }
}
