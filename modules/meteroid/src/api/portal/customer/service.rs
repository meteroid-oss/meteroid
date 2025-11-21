use crate::api::customers::mapping::customer::ServerCustomerWrapper;
use crate::api::portal::customer::error::PortalCustomerApiError;
use crate::api::portal::customer::PortalCustomerServiceComponents;
use common_domain::ids::BaseId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::portal::customer::v1::portal_customer_service_server::PortalCustomerService;
use meteroid_grpc::meteroid::portal::customer::v1::*;
use meteroid_store::domain::{InvoiceStatusEnum, OrderByRequest, PaginationRequest};
use meteroid_store::domain::enums::SubscriptionStatusEnum;
use meteroid_store::repositories::{InvoiceInterface, SubscriptionInterface};
use meteroid_store::repositories::customers::CustomersInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use tonic::{Request, Response, Status};
use common_utils::integers::ToNonNegativeU64;
use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};
use crate::api::utils::PaginationExt;

#[tonic::async_trait]
impl PortalCustomerService for PortalCustomerServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_customer_portal_overview(
        &self,
        request: Request<GetCustomerPortalOverviewRequest>,
    ) -> Result<Response<GetCustomerPortalOverviewResponse>, Status> {
        let tenant = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;

        // Get customer details
        let customer = self
            .store
            .find_customer_by_id(customer_id, tenant)
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        // Get activated subscriptions
        let subscriptions = self
            .store
            .list_subscriptions(
                tenant,
                Some(customer_id),
                None,
                Some(vec![
                    SubscriptionStatusEnum::Active,
                    SubscriptionStatusEnum::Completed,
                    SubscriptionStatusEnum::Cancelled,
                    SubscriptionStatusEnum::TrialActive,
                    SubscriptionStatusEnum::TrialExpired,
                    SubscriptionStatusEnum::Paused,
                ]),
                PaginationRequest {
                    page: 0,
                    per_page: Some(10),
                },
            )
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        let active_subscriptions: Vec<SubscriptionSummary> = subscriptions
            .items
            .into_iter()
            .filter(|s| {
                !matches!(
                    s.status,
                    SubscriptionStatusEnum::Superseded | SubscriptionStatusEnum::Completed
                )
            })
            .map(|s| {
                let status = crate::api::subscriptions::mapping::subscriptions::map_subscription_status(s.status) ;
                Ok::<_, Status>(SubscriptionSummary {
                    id: s.id.as_proto(),
                    subscription_name: s.customer_name.clone(),
                    plan_name: s.plan_name,
                    status: status.into(),
                    billing_start: s.billing_start_date.as_proto(),
                    billing_end: s.end_date.as_proto(),
                    mrr_cents: s.mrr_cents,
                    currency: s.currency,
                    trial_start: None, // TODO
                    trial_end: None,
                    next_billing_date: None,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Get recent invoices
        let invoices = self
            .store
            .list_invoices(
                tenant,
                Some(customer_id),
                None,
                None,
                None,
                OrderByRequest::IdDesc,
                PaginationRequest {
                    page: 0,
                    per_page: Some(5),
                },
            )
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        let recent_invoices: Vec<InvoiceSummary> = invoices
            .items
            .into_iter()
            .map(|inv| InvoiceSummary {
                id: inv.invoice.id.as_proto(),
                invoice_number: inv.invoice.invoice_number.clone(),
                status: crate::api::invoices::mapping::invoices::status_domain_to_server(&inv.invoice.status).into(),
                invoice_date: inv.invoice.invoice_date.as_proto(),
                due_date: inv.invoice.due_at.as_proto(),
                total_cents: inv.invoice.total.to_non_negative_u64() ,
                amount_due_cents: inv.invoice.amount_due.to_non_negative_u64() ,
                currency: inv.invoice.currency.clone(),
                plan_name: inv.invoice.plan_name.clone(),
                payment_status: crate::api::invoices::mapping::invoices::payment_status_domain_to_server(inv.invoice.payment_status).into(),
                document_sharing_key: None, // TODO
            })
            .collect();


        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        let payment_methods = customer_methods
            .into_iter()
            .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let customer_proto = ServerCustomerWrapper::try_from(customer.clone())
            .map(|v| v.0)
            .map_err(Into::<PortalCustomerApiError>::into)?;

        // Get or create payment connections for this customer
        let (card_connection_id, direct_debit_connection_id) = self
            .services
            .get_or_create_customer_connections(
                tenant,
                customer.id,
                customer.invoicing_entity_id,
            )
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        Ok(Response::new(GetCustomerPortalOverviewResponse {
            overview: Some(CustomerPortalOverview {
                customer: Some(customer_proto),
                active_subscriptions,
                recent_invoices,
                payment_methods,
                card_connection_id: card_connection_id.map(|id| id.to_string()),
                direct_debit_connection_id: direct_debit_connection_id.map(|id| id.to_string()),
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_invoices(
        &self,
        request: Request<ListInvoicesRequest>,
    ) -> Result<Response<ListInvoicesResponse>, Status> {
        let tenant = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;
        let inner = request.into_inner();

        let pagination_req = inner.pagination.into_domain();

        let invoices = self
            .store
            .list_invoices(
                tenant,
                Some(customer_id),
                None,
                None,
                None,
                OrderByRequest::IdDesc,
                pagination_req,
            )
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        Ok(Response::new(ListInvoicesResponse {
            pagination_meta: inner
                .pagination
                .into_response(invoices.total_pages, invoices.total_results),
            invoices: invoices
                .items
                .into_iter()
                .filter(|i| i.invoice.status != InvoiceStatusEnum::Draft) // TODO change list_invoice to accept an array of status
                .map(|inv| InvoiceSummary {
                    id: inv.invoice.id.as_proto(),
                    invoice_number: inv.invoice.invoice_number.clone(),
                    status: crate::api::invoices::mapping::invoices::status_domain_to_server(&inv.invoice.status).into(),
                    invoice_date: inv.invoice.invoice_date.as_proto(),
                    due_date: inv.invoice.due_at.as_proto(),
                    total_cents: inv.invoice.total.to_non_negative_u64() ,
                    amount_due_cents: inv.invoice.amount_due.to_non_negative_u64() ,
                    currency: inv.invoice.currency.clone(),
                    plan_name: inv.invoice.plan_name.clone(),
                    payment_status: crate::api::invoices::mapping::invoices::payment_status_domain_to_server(inv.invoice.payment_status).into(),
                    document_sharing_key: None, // TODO
                })
                .collect::<Vec<InvoiceSummary>>(),
        }))
    }
    //
    // #[tracing::instrument(skip_all)]
    // async fn update_customer(
    //     &self,
    //     request: Request<UpdateCustomerRequest>,
    // ) -> Result<Response<UpdateCustomerResponse>, Status> {
    //     let tenant_id = request.tenant()?;
    //     let customer_id = request.portal_resource()?.customer()?;
    //
    //     let customer = request
    //         .into_inner()
    //         .customer
    //         .ok_or(PortalCustomerApiError::MissingArgument(
    //             "customer payload missing".to_string(),
    //         ))?;
    //
    //     // Verify the customer ID matches
    //     let request_customer_id = CustomerId::from_proto(&customer.id)?;
    //     if request_customer_id != customer_id {
    //         return Err(PortalCustomerApiError::InvalidArgument(
    //             "Customer ID mismatch".to_string(),
    //         )
    //         .into());
    //     }
    //
    //     // Reuse the update logic from checkout portal
    //     use crate::api::customers::error::CustomerApiError;
    //     use crate::api::customers::mapping::customer::{
    //         DomainAddressWrapper, DomainShippingAddressWrapper,
    //     };
    //     use common_domain::ids::{BankAccountId, InvoicingEntityId};
    //     use meteroid_store::domain::CustomerPatch;
    //     use meteroid_store::repositories::customers::CustomersInterface;
    //     use rust_decimal::Decimal;
    //     use crate::api::shared::conversions::FromProtoOpt;
    //
    //     let billing_address = customer
    //         .billing_address
    //         .map(DomainAddressWrapper::try_from)
    //         .transpose()?
    //         .map(|v| v.0);
    //     let shipping_address = customer
    //         .shipping_address
    //         .map(DomainShippingAddressWrapper::try_from)
    //         .transpose()?
    //         .map(|v| v.0);
    //
    //     let customer = self
    //         .store
    //         .patch_customer(
    //             customer_id.as_uuid(),
    //             tenant_id,
    //             CustomerPatch {
    //                 id: customer_id,
    //                 name: customer.name.clone(),
    //                 alias: customer.alias.clone(),
    //                 billing_email: customer.billing_email.clone(),
    //                 invoicing_emails: customer.invoicing_emails.map(|v| v.emails),
    //                 phone: customer.phone.clone(),
    //                 balance_value_cents: customer.balance_value_cents,
    //                 invoicing_entity_id: InvoicingEntityId::from_proto_opt(
    //                     customer.invoicing_entity_id,
    //                 )?,
    //                 currency: customer.currency.clone(),
    //                 billing_address,
    //                 shipping_address,
    //                 vat_number: customer
    //                     .vat_number
    //                     .map(|v| if v.is_empty() { None } else { Some(v) }),
    //                 custom_tax_rate: Some(Decimal::from_proto_opt(customer.custom_tax_rate)?),
    //                 bank_account_id: Some(BankAccountId::from_proto_opt(customer.bank_account_id)?),
    //                 is_tax_exempt: customer.is_tax_exempt,
    //             },
    //         )
    //         .await
    //         .map_err(Into::<CustomerApiError>::into)?;
    //
    //     Ok(Response::new(UpdateCustomerResponse {
    //         customer: customer
    //             .map(ServerCustomerWrapper::try_from)
    //             .transpose()
    //             .map_err(Into::<PortalCustomerApiError>::into)?
    //             .map(|v| v.0),
    //     }))
    // }

}
