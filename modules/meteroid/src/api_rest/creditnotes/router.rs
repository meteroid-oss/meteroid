use crate::api_rest::AppState;
use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use crate::api_rest::invoices::model::BinaryFile;
use crate::errors::RestApiError;
use crate::services::storage::Prefix;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use common_domain::ids::CreditNoteId;
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use hyper::StatusCode;
use meteroid_store::repositories::CreditNoteInterface;

#[utoipa::path(
    get,
    path = "/api/v1/credit-notes/{credit_note_id}/download",
    tag = "credit-notes",
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
pub(crate) async fn download_credit_note_pdf(
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
            let data = app_state
                .object_store
                .retrieve(id, Prefix::CreditNotePdf)
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
