use crate::AppState;
use crate::invoices::mapping::domain_to_rest;
use crate::invoices::model::{
    BinaryFile, Invoice, InvoiceCustomPropertiesRequest, InvoiceListRequest, InvoiceListResponse,
};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::InvoiceId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use hyper::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::services::storage;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_rest_common::model::{PaginationExt, validate_order_by};
use meteroid_store::repositories::CreditNoteInterface;
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
pub async fn list_invoices(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<InvoiceListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let order_by = validate_order_by(
        &request.order_by,
        &[
            "created_at",
            "invoice_number",
            "customer_name",
            "amount",
            "invoice_date",
            "billing_period_start",
            "status",
            "payment_status",
        ],
        "created_at.desc",
    )
    .map_err(RestApiError::InvalidInput)?;

    let res = app_state
        .store
        .list_full_invoices(
            authorized_state.tenant_id,
            request.customer_id,
            request.subscription_id,
            request
                .statuses
                .clone()
                .map(|s| s.into_iter().map(Into::into).collect()),
            request.einvoicing_status.map(Into::into),
            None,
            Some(order_by),
            request.pagination.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error listing full invoices: {}", e);
            RestApiError::from(e)
        })?;

    let invoice_ids = res
        .items
        .iter()
        .map(|(invoice, _)| invoice.invoice.id)
        .collect::<Vec<_>>();

    let children = app_state
        .store
        .find_child_invoice_ids(authorized_state.tenant_id, &invoice_ids)
        .await
        .map_err(|e| {
            log::error!("Error resolving corrective invoices: {}", e);
            RestApiError::from(e)
        })?;

    let settled = app_state
        .store
        .invoices_settled_by_credit_notes(
            authorized_state.tenant_id,
            &res.items
                .iter()
                .map(|(invoice, _)| &invoice.invoice)
                .collect::<Vec<_>>(),
        )
        .await
        .map_err(|e| {
            log::error!("Error resolving invoices settled by credit notes: {}", e);
            RestApiError::from(e)
        })?;

    // The list shows the stable billing-period date ("Invoice date") and does not project draft
    // emission dates — that's a detail-view concern.
    let invoice_items = res.items;

    let items = invoice_items
        .into_iter()
        .map(|(invoice, txs)| {
            let child_id = children.get(&invoice.invoice.id).copied();
            let settled_by_credit_notes = settled.contains(&invoice.invoice.id);
            domain_to_rest(invoice.invoice, txs, child_id, settled_by_credit_notes)
        })
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
pub async fn get_invoice_by_id(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(invoice_id): Path<InvoiceId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    // Detailed rather than by-id: it joins the customer, which is what resolves a draft's
    // identity. The raw row would serve a stale bill-to here while gRPC GetInvoice serves the
    // live one.
    let res = app_state
        .store
        .get_detailed_invoice_by_id(authorized_state.tenant_id, invoice_id)
        .await
        .map_err(|e| {
            log::error!("Error getting invoice by id: {}", e);
            RestApiError::StoreError
        })?
        .invoice;

    // Drafts display the dates finalization would stamp today; backdated (historical import)
    // subscriptions and manual invoices keep their stored dates.
    let mut res = res;
    app_state
        .store
        .project_draft_emission_dates(&mut res)
        .await
        .map_err(|e| {
            log::error!("Error projecting invoice emission dates: {}", e);
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

    let child_invoice_id = app_state
        .store
        .find_child_invoice_id(authorized_state.tenant_id, invoice_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving corrective invoice: {}", e);
            RestApiError::StoreError
        })?;

    let settled_by_credit_notes = settled_by_credit_notes(&app_state, &res).await?;
    let rest_model = domain_to_rest(res, transactions, child_invoice_id, settled_by_credit_notes)?;

    Ok(Json(rest_model))
}

/// Update invoice custom properties
///
/// Merge custom property values onto an invoice (send a key with `null` to remove it).
/// Values are validated against the tenant's `INVOICE` property definitions. Allowed at any
/// status — custom properties are external workflow metadata and stay editable after the invoice
/// is finalized.
#[utoipa::path(
    patch,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/custom-properties",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID", example = "inv_123"),
    ),
    request_body(content = InvoiceCustomPropertiesRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Updated invoice", body = Invoice),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Invoice not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn patch_invoice_custom_properties(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(invoice_id): Path<InvoiceId>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<InvoiceCustomPropertiesRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .patch_invoice_custom_properties(
            authorized_state.tenant_id,
            invoice_id,
            payload.custom_properties,
        )
        .await
        .map_err(|e| {
            log::error!("Error patching invoice custom properties: {}", e);
            RestApiError::from(e)
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

    let child_invoice_id = app_state
        .store
        .find_child_invoice_id(authorized_state.tenant_id, invoice_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving corrective invoice: {}", e);
            RestApiError::StoreError
        })?;

    let settled_by_credit_notes = settled_by_credit_notes(&app_state, &res).await?;
    Ok(Json(domain_to_rest(
        res,
        transactions,
        child_invoice_id,
        settled_by_credit_notes,
    )?))
}

/// Refresh invoice
///
/// Recompute a draft invoice against current usage, credits, coupons and tax, and return it.
/// Drafts are also refreshed periodically in the background; use this to force it, e.g. after
/// ingesting late events. Rejected while a payment for the invoice is in progress or when the
/// invoice was merged into a consolidated parent.
#[utoipa::path(
    post,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/refresh",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID", example = "inv_123"),
    ),
    responses(
        (status = 200, description = "Refreshed invoice", body = Invoice),
        (status = 400, description = "Invoice cannot be refreshed", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Invoice not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn refresh_invoice(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(invoice_id): Path<InvoiceId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let detailed = app_state
        .services
        .refresh_invoice_data(invoice_id, authorized_state.tenant_id)
        .await
        .map_err(|e| {
            log::error!("Error refreshing invoice: {}", e);
            RestApiError::from(e)
        })?;

    let mut res = detailed.invoice;
    app_state
        .store
        .project_draft_emission_dates(&mut res)
        .await
        .map_err(|e| {
            log::error!("Error projecting invoice emission dates: {}", e);
            RestApiError::StoreError
        })?;

    let child_invoice_id = app_state
        .store
        .find_child_invoice_id(authorized_state.tenant_id, invoice_id)
        .await
        .map_err(|e| {
            log::error!("Error resolving corrective invoice: {}", e);
            RestApiError::StoreError
        })?;

    let settled_by_credit_notes = settled_by_credit_notes(&app_state, &res).await?;
    Ok(Json(domain_to_rest(
        res,
        detailed.transactions,
        child_invoice_id,
        settled_by_credit_notes,
    )?))
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
pub async fn download_invoice_pdf(
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
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
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

/// Download invoice e-invoice XML
///
/// Download the structured e-invoice (EN 16931 XML) issued with an invoice. For
/// Factur-X the same XML is also embedded in the PDF.
#[utoipa::path(
    get,
    tag = "Invoices",
    path = "/api/v1/invoices/{invoice_id}/xml",
    params(
        ("invoice_id" = InvoiceId, Path, description = "Invoice ID", example = "inv_123"),
    ),
    responses(
        (status = 200, description = "Invoice e-invoice XML", content_type = "application/xml", body = inline(BinaryFile)),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Invoice not found, or not issued as an e-invoice", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn download_invoice_xml(
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

    match invoice.xml_document_id {
        Some(id) => {
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
                .await
                .map_err(|e| {
                    log::error!("Error getting invoice XML by id: {}", e);
                    RestApiError::ObjectStoreError
                })?;
            Ok(crate::files::xml_download(data, &invoice.invoice_number))
        }
        None => Ok(crate::files::no_xml()),
    }
}

async fn settled_by_credit_notes(
    app_state: &AppState,
    invoice: &meteroid_store::domain::Invoice,
) -> Result<bool, RestApiError> {
    Ok(app_state
        .store
        .invoices_settled_by_credit_notes(invoice.tenant_id, &[invoice])
        .await
        .map_err(|e| {
            log::error!(
                "Error resolving the credit-note settlement of an invoice: {}",
                e
            );
            RestApiError::from(e)
        })?
        .contains(&invoice.id))
}
