use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::invoices::v1::{
    invoices_service_server::InvoicesService, list_invoices_request::SortBy, GetInvoiceRequest,
    GetInvoiceResponse, Invoice, ListInvoicesRequest, ListInvoicesResponse, PreviewInvoiceRequest,
    PreviewInvoiceResponse, RefreshInvoiceDataRequest, RefreshInvoiceDataResponse,
};
use meteroid_store::domain;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::repositories::InvoiceInterface;

use crate::api::invoices::error::InvoiceApiError;
use crate::api::utils::parse_uuid;
use crate::api::utils::PaginationExt;

use super::{mapping, InvoiceServiceComponents};

#[tonic::async_trait]
impl InvoicesService for InvoiceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_invoices(
        &self,
        request: Request<ListInvoicesRequest>,
    ) -> Result<Response<ListInvoicesResponse>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let customer_id = inner
            .customer_id
            .map(|c| parse_uuid(&c, "customer_id").unwrap());

        let pagination_req = domain::PaginationRequest {
            page: inner.pagination.as_ref().map(|p| p.offset).unwrap_or(0),
            per_page: inner.pagination.as_ref().map(|p| p.limit),
        };

        let order_by = match inner.sort_by.try_into() {
            Ok(SortBy::DateAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::DateDesc) => OrderByRequest::DateDesc,
            Ok(SortBy::IdAsc) => OrderByRequest::IdAsc,
            Ok(SortBy::IdDesc) => OrderByRequest::IdDesc,
            Err(_) => OrderByRequest::DateDesc,
        };

        let res = self
            .store
            .list_invoices(
                tenant_id,
                customer_id,
                mapping::invoices::status_server_to_domain(inner.status),
                inner.search,
                order_by,
                pagination_req,
            )
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = ListInvoicesResponse {
            pagination_meta: inner.pagination.into_response(res.total_results as u32),
            invoices: res
                .items
                .into_iter()
                .map(mapping::invoices::domain_to_server)
                .collect::<Vec<Invoice>>(),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn get_invoice(
        &self,
        request: Request<GetInvoiceRequest>,
    ) -> Result<Response<GetInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let invoice = self
            .store
            .find_invoice_by_id(tenant_id, parse_uuid(&req.id, "id")?)
            .await
            .and_then(mapping::invoices::domain_invoice_with_plan_details_to_server)
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = GetInvoiceResponse {
            invoice: Some(invoice),
        };

        Ok(Response::new(response))
    }

    async fn preview_invoice_html(
        &self,
        request: Request<PreviewInvoiceRequest>,
    ) -> Result<Response<PreviewInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let html = self
            .html_rendering
            .preview_invoice_html(parse_uuid(&req.id, "invoice_id")?, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = PreviewInvoiceResponse { html };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn refresh_invoice_data(
        &self,
        request: Request<RefreshInvoiceDataRequest>,
    ) -> Result<Response<RefreshInvoiceDataResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let invoice = self
            .store
            .refresh_invoice_data(parse_uuid(&req.id, "id")?, tenant_id)
            .await
            .and_then(mapping::invoices::domain_invoice_with_plan_details_to_server)
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = RefreshInvoiceDataResponse {
            invoice: Some(invoice),
        };

        Ok(Response::new(response))
    }
}
