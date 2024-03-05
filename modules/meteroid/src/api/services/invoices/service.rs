use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::invoices::v1::{
    invoices_service_server::InvoicesService, list_invoices_request::SortBy, GetInvoiceRequest,
    GetInvoiceResponse, ListInvoicesRequest, ListInvoicesResponse,
};
use meteroid_repository as db;
use meteroid_repository::Params;

use crate::api::services::invoices::error::InvoiceServiceError;
use crate::api::services::utils::parse_uuid;
use crate::api::services::utils::PaginationExt;

use super::{mapping, DbService};

#[tonic::async_trait]
impl InvoicesService for DbService {
    #[tracing::instrument(skip_all)]
    async fn list_invoices(
        &self,
        request: Request<ListInvoicesRequest>,
    ) -> Result<Response<ListInvoicesResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::invoices::ListTenantInvoicesParams {
            tenant_id,
            limit: req.pagination.limit(),
            offset: req.pagination.offset(),
            status: mapping::invoices::status_server_to_db(req.status),
            order_by: match req.order_by.try_into() {
                Ok(SortBy::DateAsc) => "DATE_ASC",
                Ok(SortBy::DateDesc) => "DATE_DESC",
                Ok(SortBy::IdAsc) => "ID_ASC",
                Ok(SortBy::IdDesc) => "ID_DESC",
                Err(_) => "DATE_DESC",
            },
            search: req.search,
            customer_id: req
                .customer_id
                .map(|c| parse_uuid(&c, "customer_id").unwrap()),
        };

        let invoices = db::invoices::list_tenant_invoices()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                InvoiceServiceError::DatabaseError(
                    format!("Unable to list invoices of {}", tenant_id),
                    e,
                )
            })?;

        let total = invoices.first().map(|p| p.total_count).unwrap_or(0);

        let response = ListInvoicesResponse {
            pagination_meta: req.pagination.into_response(total as u32),
            invoices: invoices
                .into_iter()
                .map(|f| mapping::invoices::db_to_server_list(f))
                .collect::<Vec<_>>(),
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
        let connection = self.get_connection().await?;

        let params = db::invoices::GetTenantInvoiceByIdParams {
            tenant_id,
            id: parse_uuid(&req.id, "id")?,
        };

        let invoice = db::invoices::get_tenant_invoice_by_id()
            .params(&connection, &params)
            .one()
            .await
            .map_err(|e| {
                InvoiceServiceError::DatabaseError(
                    format!("Unable to get invoice {} of {}", req.id, tenant_id),
                    e,
                )
            })?;

        let response = GetInvoiceResponse {
            invoice: Some(mapping::invoices::db_to_server(invoice)),
        };

        Ok(Response::new(response))
    }
}
