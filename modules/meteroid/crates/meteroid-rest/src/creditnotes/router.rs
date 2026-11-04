use crate::AppState;
use crate::creditnotes::mapping::domain_to_rest;
use crate::creditnotes::model::{
    CreditNote, CreditNoteCustomPropertiesRequest, CreditNoteListRequest, CreditNoteListResponse,
};
use crate::invoices::model::BinaryFile;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use axum_valid::Valid;
use common_domain::ids::CreditNoteId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use hyper::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::services::storage;
use meteroid_rest_common::QueryParams;
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_rest_common::model::{PaginationExt, validate_order_by};
use meteroid_store::repositories::CreditNoteInterface;

/// List credit notes
///
/// List a tenant's credit notes, optionally filtered by customer, invoice or status.
#[utoipa::path(
    get,
    tag = "Credit Notes",
    path = "/api/v1/credit-notes",
    params(CreditNoteListRequest),
    responses(
        (status = 200, description = "List of credit notes", body = CreditNoteListResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn list_credit_notes(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Valid(QueryParams(request)): Valid<QueryParams<CreditNoteListRequest>>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let order_by = validate_order_by(
        &request.order_by,
        &["created_at", "credit_note_number", "total", "status"],
        "created_at.desc",
    )
    .map_err(RestApiError::InvalidInput)?;

    let res = app_state
        .store
        .list_credit_notes(
            authorized_state.tenant_id,
            request.customer_id,
            request.invoice_id,
            request.status.map(Into::into),
            request.search.clone(),
            vec![],
            Some(order_by),
            request.pagination.into(),
        )
        .await
        .map_err(|e| {
            log::error!("Error listing credit notes: {}", e);
            RestApiError::from(e)
        })?;

    let items = res
        .items
        .into_iter()
        .map(domain_to_rest)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Json(CreditNoteListResponse {
        data: items,
        pagination_meta: request
            .pagination
            .into_response(res.total_pages, res.total_results),
    }))
}

/// Get credit note
///
/// Retrieve a single credit note by ID.
#[utoipa::path(
    get,
    tag = "Credit Notes",
    path = "/api/v1/credit-notes/{credit_note_id}",
    params(
        ("credit_note_id" = CreditNoteId, Path, description = "Credit Note ID", example = "cn_123"),
    ),
    responses(
        (status = 200, description = "Credit note details", body = CreditNote),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Credit note not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn get_credit_note_by_id(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(credit_note_id): Path<CreditNoteId>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .get_credit_note_by_id(authorized_state.tenant_id, credit_note_id)
        .await
        .map_err(|e| {
            log::error!("Error getting credit note by id: {}", e);
            RestApiError::from(e)
        })?;

    Ok(Json(domain_to_rest(res)?))
}

/// Update credit note custom properties
///
/// Merge custom property values onto a credit note (send a key with `null` to remove it).
/// Values are validated against the tenant's `CREDIT_NOTE` property definitions. Allowed at any
/// status — custom properties are external workflow metadata and stay editable after the credit
/// note is finalized.
#[utoipa::path(
    patch,
    tag = "Credit Notes",
    path = "/api/v1/credit-notes/{credit_note_id}/custom-properties",
    params(
        ("credit_note_id" = CreditNoteId, Path, description = "Credit Note ID", example = "cn_123"),
    ),
    request_body(content = CreditNoteCustomPropertiesRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Updated credit note", body = CreditNote),
        (status = 400, description = "Bad request", body = RestErrorResponse),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Credit note not found", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn patch_credit_note_custom_properties(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(credit_note_id): Path<CreditNoteId>,
    State(app_state): State<AppState>,
    Valid(Json(payload)): Valid<Json<CreditNoteCustomPropertiesRequest>>,
) -> Result<impl IntoResponse, RestApiError> {
    let res = app_state
        .store
        .patch_credit_note_custom_properties(
            authorized_state.tenant_id,
            credit_note_id,
            payload.custom_properties,
        )
        .await
        .map_err(|e| {
            log::error!("Error patching credit note custom properties: {}", e);
            RestApiError::from(e)
        })?;

    Ok(Json(domain_to_rest(res)?))
}

#[utoipa::path(
    get,
    path = "/api/v1/credit-notes/{credit_note_id}/download",
    tag = "Credit Notes",
    params(
        ("credit_note_id" = CreditNoteId, Path, description = "Credit Note ID", example = "cn_123"),
    ),
    responses(
        (status = 200, description = "Credit Note PDF", content_type = "application/pdf", body = inline(BinaryFile)),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Credit note not found or PDF not available", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn download_credit_note_pdf(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(credit_note_id): Path<CreditNoteId>,
    State(app_state): State<AppState>,
) -> Result<Response, RestApiError> {
    let credit_note = app_state
        .store
        .get_credit_note_by_id(authorized_state.tenant_id, credit_note_id)
        .await
        .map_err(|e| {
            log::error!("Error getting credit note by id: {}", e);
            RestApiError::StoreError
        })?;

    match credit_note.pdf_document_id {
        Some(id) => {
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
                .await
                .map_err(|e| {
                    log::error!("Error getting credit note file by id: {}", e);
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

/// Download credit note e-invoice XML
///
/// Download the structured e-invoice (EN 16931 XML) issued with a credit note. For
/// Factur-X the same XML is also embedded in the PDF.
#[utoipa::path(
    get,
    path = "/api/v1/credit-notes/{credit_note_id}/xml",
    tag = "Credit Notes",
    params(
        ("credit_note_id" = CreditNoteId, Path, description = "Credit Note ID", example = "cn_123"),
    ),
    responses(
        (status = 200, description = "Credit Note e-invoice XML", content_type = "application/xml", body = inline(BinaryFile)),
        (status = 401, description = "Unauthorized", body = RestErrorResponse),
        (status = 404, description = "Credit note not found, or not issued as an e-invoice", body = RestErrorResponse),
        (status = 500, description = "Internal error", body = RestErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[axum::debug_handler]
pub async fn download_credit_note_xml(
    Extension(authorized_state): Extension<AuthorizedAsTenant>,
    Path(credit_note_id): Path<CreditNoteId>,
    State(app_state): State<AppState>,
) -> Result<Response, RestApiError> {
    let credit_note = app_state
        .store
        .get_credit_note_by_id(authorized_state.tenant_id, credit_note_id)
        .await
        .map_err(|e| {
            log::error!("Error getting credit note by id: {}", e);
            RestApiError::StoreError
        })?;

    match credit_note.xml_document_id {
        Some(id) => {
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
                .await
                .map_err(|e| {
                    log::error!("Error getting credit note XML by id: {}", e);
                    RestApiError::ObjectStoreError
                })?;
            Ok(crate::files::xml_download(
                data,
                &credit_note.credit_note_number,
            ))
        }
        None => Ok(crate::files::no_xml()),
    }
}
