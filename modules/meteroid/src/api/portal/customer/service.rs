use crate::api::customers::mapping::customer::ServerCustomerWrapper;
use crate::api::portal::customer::PortalCustomerServiceComponents;
use crate::api::portal::customer::error::PortalCustomerApiError;
use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};
use crate::api::utils::PaginationExt;
use common_grpc::middleware::server::auth::RequestExt;
use common_utils::integers::ToNonNegativeU64;
use meteroid_grpc::meteroid::portal::customer::v1::portal_customer_service_server::PortalCustomerService;
use meteroid_grpc::meteroid::portal::customer::v1::*;
use meteroid_store::domain::enums::SubscriptionStatusEnum;
use meteroid_store::domain::{InvoiceStatusEnum, OrderByRequest, PaginationRequest};
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::repositories::{InvoiceInterface, SubscriptionInterface};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl PortalCustomerService for PortalCustomerServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_customer_portal_overview(
        &self,
        request: Request<GetCustomerPortalOverviewRequest>,
    ) -> Result<Response<GetCustomerPortalOverviewResponse>, Status> {
        let tenant = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;

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
                    // SubscriptionStatusEnum::Completed,
                    // SubscriptionStatusEnum::Cancelled,
                    SubscriptionStatusEnum::TrialActive,
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
                let status =
                    crate::api::subscriptions::mapping::subscriptions::map_subscription_status(
                        s.status,
                    );
                Ok::<_, Status>(SubscriptionSummary {
                    id: s.id.as_proto(),
                    subscription_name: s.customer_name.clone(),
                    plan_name: s.plan_name,
                    status: status.into(),
                    billing_start: s.billing_start_date.as_proto(),
                    billing_end: s.end_date.as_proto(),
                    mrr_cents: s.mrr_cents,
                    currency: s.currency,
                    trial_start: None, // TODO drop (status + next date)
                    trial_end: None,
                    next_billing_date: s.current_period_end.as_proto(), // TODO next action (cancels, ends, renews)
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
                status: crate::api::invoices::mapping::invoices::status_domain_to_server(
                    &inv.invoice.status,
                )
                .into(),
                invoice_date: inv.invoice.invoice_date.as_proto(),
                due_date: inv.invoice.due_at.as_proto(),
                total_cents: inv.invoice.total.to_non_negative_u64(),
                amount_due_cents: inv.invoice.amount_due.to_non_negative_u64(),
                currency: inv.invoice.currency.clone(),
                plan_name: inv.invoice.plan_name.clone(),
                payment_status:
                    crate::api::invoices::mapping::invoices::payment_status_domain_to_server(
                        inv.invoice.payment_status,
                    )
                    .into(),
                document_sharing_key: None, // TODO
            })
            .collect();

        // Resolve payment methods based on customer's invoicing entity (None defaults to Online mode)
        let resolved = self
            .services
            .resolve_subscription_payment_methods(tenant, None, &customer)
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        let customer_methods = self
            .store
            .list_payment_methods_by_customer(&tenant, &customer.id)
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        let payment_methods = resolved
            .filter_payment_methods(customer_methods)
            .into_iter()
            .map(crate::api::customers::mapping::customer_payment_method::domain_to_server)
            .collect();

        let customer_proto = ServerCustomerWrapper::try_from(customer.clone())
            .map(|v| v.0)
            .map_err(Into::<PortalCustomerApiError>::into)?;

        let card_connection_id = resolved.card_connection_id;
        let direct_debit_connection_id = resolved.direct_debit_connection_id;

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant, Some(customer.invoicing_entity_id))
            .await
            .map_err(Into::<PortalCustomerApiError>::into)?;

        let invoicing_entity_logo_url = invoicing_entity
            .logo_attachment_id
            .as_ref()
            .map(|logo_id| format!("{}/files/v1/logo/{}", self.rest_api_external_url, logo_id));

        Ok(Response::new(GetCustomerPortalOverviewResponse {
            overview: Some(CustomerPortalOverview {
                customer: Some(customer_proto),
                active_subscriptions,
                recent_invoices,
                payment_methods,
                card_connection_id: card_connection_id.map(|id| id.to_string()),
                direct_debit_connection_id: direct_debit_connection_id.map(|id| id.to_string()),
                invoicing_entity_name: Some(invoicing_entity.legal_name),
                invoicing_entity_logo_url,
                invoicing_entity_brand_color: invoicing_entity.brand_color,
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
                    status: crate::api::invoices::mapping::invoices::status_domain_to_server(
                        &inv.invoice.status,
                    )
                    .into(),
                    invoice_date: inv.invoice.invoice_date.as_proto(),
                    due_date: inv.invoice.due_at.as_proto(),
                    total_cents: inv.invoice.total.to_non_negative_u64(),
                    amount_due_cents: inv.invoice.amount_due.to_non_negative_u64(),
                    currency: inv.invoice.currency.clone(),
                    plan_name: inv.invoice.plan_name.clone(),
                    payment_status:
                        crate::api::invoices::mapping::invoices::payment_status_domain_to_server(
                            inv.invoice.payment_status,
                        )
                        .into(),
                    document_sharing_key: None, // TODO
                })
                .collect::<Vec<InvoiceSummary>>(),
        }))
    }
}
