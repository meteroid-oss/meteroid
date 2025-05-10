use super::{InvoiceServiceComponents, mapping};
use crate::api::invoices::error::InvoiceApiError;
use crate::api::utils::PaginationExt;
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::invoices::v1::{
    GetInvoiceRequest, GetInvoiceResponse, Invoice, ListInvoicesRequest, ListInvoicesResponse,
    PreviewInvoiceRequest, PreviewInvoiceResponse, RefreshInvoiceDataRequest,
    RefreshInvoiceDataResponse, RequestPdfGenerationRequest, RequestPdfGenerationResponse,
    SyncToPennylaneRequest, SyncToPennylaneResponse, invoices_service_server::InvoicesService,
    list_invoices_request::SortBy,
};
use meteroid_store::domain;
use meteroid_store::domain::OrderByRequest;
use meteroid_store::domain::pgmq::{InvoicePdfRequestEvent, PgmqMessageNew, PgmqQueue};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl InvoicesService for InvoiceServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_invoices(
        &self,
        request: Request<ListInvoicesRequest>,
    ) -> Result<Response<ListInvoicesResponse>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto_opt(inner.customer_id)?;
        let subscription_id = SubscriptionId::from_proto_opt(inner.subscription_id)?;

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
                subscription_id,
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
            .find_invoice_by_id(tenant_id, InvoiceId::from_proto(&req.id)?)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_plan_details_to_server(
                    inv,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = GetInvoiceResponse {
            invoice: Some(invoice),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_invoice_html(
        &self,
        request: Request<PreviewInvoiceRequest>,
    ) -> Result<Response<PreviewInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let html = self
            .preview_rendering
            .preview_invoice(InvoiceId::from_proto(&req.id)?, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = PreviewInvoiceResponse { html };

        Ok(Response::new(response))
    }

    // for demo & local use when the worker was not started initially
    #[tracing::instrument(skip_all)]
    async fn request_pdf_generation(
        &self,
        request: Request<RequestPdfGenerationRequest>,
    ) -> Result<Response<RequestPdfGenerationResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoice = self
            .store
            .find_invoice_by_id(tenant_id, InvoiceId::from_proto(&req.id)?)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let pgmq_msg_new: PgmqMessageNew = InvoicePdfRequestEvent::new(invoice.invoice.id)
            .try_into()
            .map_err(Into::<InvoiceApiError>::into)?;

        // check if already generated ?
        self.store
            .pgmq_send_batch(PgmqQueue::InvoicePdfRequest, vec![pgmq_msg_new])
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = RequestPdfGenerationResponse {};

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
            .refresh_invoice_data(InvoiceId::from_proto(&req.id)?, tenant_id)
            .await
            .and_then(|inv| {
                mapping::invoices::domain_invoice_with_plan_details_to_server(
                    inv,
                    self.jwt_secret.clone(),
                )
            })
            .map_err(Into::<InvoiceApiError>::into)?;

        let response = RefreshInvoiceDataResponse {
            invoice: Some(invoice),
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn sync_to_pennylane(
        &self,
        request: Request<SyncToPennylaneRequest>,
    ) -> Result<Response<SyncToPennylaneResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let ids = req
            .invoice_ids
            .iter()
            .map(InvoiceId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        self.store
            .sync_invoices_to_pennylane(ids, tenant_id)
            .await
            .map_err(Into::<InvoiceApiError>::into)?;

        Ok(Response::new(SyncToPennylaneResponse {}))
    }
}
