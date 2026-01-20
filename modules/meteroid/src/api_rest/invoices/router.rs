use crate::api_rest::AppState;
use crate::api_rest::QueryParams;
use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use crate::api_rest::invoices::mapping::{domain_to_rest, map_status_from_rest};
use crate::api_rest::invoices::model::{
    BinaryFile, Invoice, InvoiceListRequest, InvoiceListResponse,
};
use crate::api_rest::model::PaginationExt;
use crate::errors::RestApiError;
use crate::services::storage::Prefix;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::InvoiceId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use hyper::StatusCode;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

/// List invoices
///
/// List invoices with optional filtering by customer, subscription, or status.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v1/invoices",
    params(
        InvoiceListRequest
    ),
    responses(
        (status = 200, description = "List of invoices", body = InvoiceListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn list_invoices(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<InvoiceListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .list_full_invoices(
            authorized_state.tenant_id,
            request.customer_id,
            request.subscription_id,
            request
                .statuses
                .clone()
                .map(|s| s.into_iter().map(map_status_from_rest).collect()),
            None,
            meteroid_store::domain::OrderByRequest::IdDesc,
            request.pagination.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error listing full invoices: {}", e);
            RestApiError::from(e)
        })?;

    let items = res
        .items
        .into_iter()
        .map(|(invoice, txs)| domain_to_rest(invoice.invoice, txs))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(InvoiceListResponse {
        data: items,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Get invoice
///
/// Retrieve a single invoice with its payment transactions.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID", example = "inv_123"),
    ),
    responses(
        (status = 200, description = "Invoice details", body = Invoice),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Invoice not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn get_invoice_by_id(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(invoice_id): Path<InvoiceId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .get_invoice_by_id(authorized_state.tenant_id, invoice_id)
        .await
        .map_err(|e| {
            log::error!("Error getting invoice by id: {}", e);
            RestApiError::StoreError
        })?;

    let transactions = app_state
        .store
        .list_payment_tx_by_invoice_id(authorized_state.tenant_id, invoice_id)
        .await
        .map_err(|e| {
            log::error!("Error getting transactions for invoice: {}", e);
            RestApiError::StoreError
        })?
        .into_iter()
        .map(|tx| tx.transaction)
        .collect::<Vec<_>>();

    let rest_model = domain_to_rest(res, transactions)?;

    Ok(Json(rest_model))
}

/// Download invoice PDF
///
/// Download the PDF document for an invoice.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/download",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID", example = "inv_123"),
    ),
    responses(
        (status = 200, description = "Invoice PDF", content_type = "application/pdf", body = inline(BinaryFile)),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Invoice not found or PDF not available", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub(crate) async fn download_invoice_pdf(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(invoice_id): Path<InvoiceId>,
    State(app_state): State<AppState>,
) -> Result<Response, RestApiError> {
    let invoice = app_state
        .store
        .get_invoice_by_id(authorized_state.tenant_id, invoice_id)
        .await
        .map_err(|e| {
            log::error!("Error getting invoice by id: {}", e);
            RestApiError::StoreError
        })?;

    match invoice.pdf_document_id {
        Some(id) => {
            let data = app_state
                .object_store
                .retrieve(id, Prefix::InvoicePdf)
                .await
                .map_err(|e| {
                    log::error!("Error getting invoice file by id: {}", e);
                    RestApiError::ObjectStoreError
                })?;

            Ok((
                StatusCode::OK,
                [
                    ("Content-Type", "application/pdf"),
                    ("Content-Disposition", "inline"),
                ],
                data,
            )
                .into_response())
        }
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(RestErrorResponse {
                code: ErrorCode::NotFound,
                message: "No attached PDF. Generation may be pending".to_string(),
            }),
        )
            .into_response()),
    }
}
