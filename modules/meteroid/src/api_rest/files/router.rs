use super::AppState;
use std::io::Cursor;

use axum::extract::Query;
use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use hyper::StatusCode;

use crate::errors;

use crate::api::sharable::ShareableEntityClaims;
use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use crate::services::storage::Prefix;
use common_domain::ids::{
    BaseId, BatchJobId, CreditNoteId, EntityActivityId, InvoiceId, StoredDocumentId,
};
use error_stack::{Report, ResultExt};
use image::ImageFormat::Png;
use jsonwebtoken::{DecodingKey, Validation, decode};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::batch_jobs::BatchJobsInterface;
use meteroid_store::repositories::entity_activity::EntityActivityInterfaceEmail;
use secrecy::ExposeSecret;
use serde::Deserialize;

#[axum::debug_handler]
pub async fn get_logo(
    Path(uuid): Path<StoredDocumentId>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_logo_handler(uuid, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling logo: {e}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_logo_handler(
    image_uuid: StoredDocumentId,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    let data = app_state
        .object_store
        .retrieve(image_uuid, Prefix::ImageLogo)
        .await
        .change_context(errors::RestApiError::ObjectStoreError)?;

    // resize
    let mut img =
        image::load_from_memory(&data).change_context(errors::RestApiError::ImageLoadingError)?;
    img = img.resize(350, 350, image::imageops::FilterType::Nearest);
    let mut buffer = Vec::new();
    img.write_to(&mut Cursor::new(&mut buffer), Png)
        .change_context(errors::RestApiError::ImageLoadingError)?;

    let data = bytes::Bytes::from(buffer);

    Ok((StatusCode::OK, [("Content-Type", "image/png")], data).into_response())
}

#[derive(Deserialize)]
pub struct TokenParams {
    token: String,
}

#[axum::debug_handler]
pub async fn get_invoice_pdf(
    Path(uid): Path<InvoiceId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_invoice_pdf_handler(uid, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling invoice PDF: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_invoice_pdf_handler(
    invoice_uid: InvoiceId,
    token: TokenParams,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    let claims = decode::<ShareableEntityClaims>(
        &token.token,
        &DecodingKey::from_secret(app_state.jwt_secret.expose_secret().as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| Report::new(errors::RestApiError::Unauthorized))?
    .claims;

    let invoice = app_state
        .store
        .get_invoice_by_id(claims.tenant_id, claims.entity_id.into())
        .await
        .change_context(errors::RestApiError::StoreError)?;

    if invoice.id != invoice_uid {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }
    match invoice.pdf_document_id {
        Some(id) => {
            let data = app_state
                .object_store
                .retrieve(id, Prefix::InvoicePdf)
                .await
                .change_context(errors::RestApiError::ObjectStoreError)?;

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

#[axum::debug_handler]
pub async fn get_credit_note_pdf(
    Path(uid): Path<CreditNoteId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_credit_note_pdf_handler(uid, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling credit note PDF: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_credit_note_pdf_handler(
    credit_note_uid: CreditNoteId,
    token: TokenParams,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    let claims = decode::<ShareableEntityClaims>(
        &token.token,
        &DecodingKey::from_secret(app_state.jwt_secret.expose_secret().as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| Report::new(errors::RestApiError::Unauthorized))?
    .claims;

    let credit_note = app_state
        .store
        .get_credit_note_by_id(claims.tenant_id, claims.entity_id.into())
        .await
        .change_context(errors::RestApiError::StoreError)?;

    if credit_note.id != credit_note_uid {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }
    match credit_note.pdf_document_id {
        Some(id) => {
            let data = app_state
                .object_store
                .retrieve(id, Prefix::CreditNotePdf)
                .await
                .change_context(errors::RestApiError::ObjectStoreError)?;

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

/// Audit attachment shape stored in `sent_email.attachments`. `id`/`kind` are
/// absent on rows written before attachment object ids were tracked.
#[derive(Deserialize)]
struct StoredEmailAttachment {
    filename: String,
    id: Option<String>,
    kind: Option<String>,
}

fn prefix_for_attachment_kind(kind: &str) -> Option<Prefix> {
    match kind {
        "invoice_pdf" => Some(Prefix::InvoicePdf),
        "receipt_pdf" => Some(Prefix::ReceiptPdf),
        "credit_note_pdf" => Some(Prefix::CreditNotePdf),
        _ => None,
    }
}

#[axum::debug_handler]
pub async fn get_email_attachment(
    Path((activity_id, doc_id)): Path<(EntityActivityId, StoredDocumentId)>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_email_attachment_handler(activity_id, doc_id, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling email attachment: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_email_attachment_handler(
    activity_id: EntityActivityId,
    doc_id: StoredDocumentId,
    token: TokenParams,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    let claims = decode::<ShareableEntityClaims>(
        &token.token,
        &DecodingKey::from_secret(app_state.jwt_secret.expose_secret().as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| Report::new(errors::RestApiError::Unauthorized))?
    .claims;

    if claims.entity_id != activity_id.as_uuid() {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }

    let email = app_state
        .store
        .get_sent_email(claims.tenant_id, activity_id)
        .await
        .change_context(errors::RestApiError::StoreError)?;

    // The recorded attachment list is the authority on what is downloadable:
    // only objects this email actually carried are reachable.
    let stored: Vec<StoredEmailAttachment> = email
        .attachments
        .map(|v| serde_json::from_value(v).unwrap_or_default())
        .unwrap_or_default();

    let target_id = doc_id.as_base62();
    let attachment = stored
        .into_iter()
        .find(|a| a.id.as_deref() == Some(target_id.as_str()))
        .ok_or_else(|| Report::new(errors::RestApiError::NotFound))?;

    let prefix = attachment
        .kind
        .as_deref()
        .and_then(prefix_for_attachment_kind)
        .ok_or_else(|| Report::new(errors::RestApiError::NotFound))?;

    let data = app_state
        .object_store
        .retrieve(doc_id, prefix)
        .await
        .change_context(errors::RestApiError::ObjectStoreError)?;

    Ok((
        StatusCode::OK,
        [
            ("Content-Type", "application/pdf".to_string()),
            (
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", attachment.filename),
            ),
        ],
        data,
    )
        .into_response())
}

#[axum::debug_handler]
pub async fn get_batch_job_error_csv(
    Path(batch_job_id): Path<BatchJobId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_batch_job_error_csv_handler(batch_job_id, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling batch job error CSV: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

#[axum::debug_handler]
pub async fn get_batch_job_input_file(
    Path(batch_job_id): Path<BatchJobId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_batch_job_input_file_handler(batch_job_id, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling batch job input file: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_batch_job_input_file_handler(
    batch_job_id: BatchJobId,
    token: TokenParams,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    let claims = decode::<ShareableEntityClaims>(
        &token.token,
        &DecodingKey::from_secret(app_state.jwt_secret.expose_secret().as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| Report::new(errors::RestApiError::Unauthorized))?
    .claims;

    let detail = app_state
        .store
        .get_batch_job(claims.entity_id.into(), claims.tenant_id)
        .await
        .change_context(errors::RestApiError::StoreError)?;

    if detail.job.id != batch_job_id {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }

    let input_key = detail
        .job
        .input_source_key
        .ok_or(Report::new(errors::RestApiError::NotFound))?;

    let doc_id: StoredDocumentId = input_key
        .parse()
        .map_err(|_| Report::new(errors::RestApiError::StoreError))?;

    let data = app_state
        .object_store
        .retrieve(
            doc_id,
            Prefix::BatchJobInput {
                tenant_id: claims.tenant_id,
            },
        )
        .await
        .change_context(errors::RestApiError::ObjectStoreError)?;

    let filename = detail
        .job
        .input_file_name
        .unwrap_or_else(|| format!("import-{}.csv", batch_job_id));

    Ok((
        StatusCode::OK,
        [
            ("Content-Type", "text/csv; charset=utf-8"),
            (
                "Content-Disposition",
                &format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        data,
    )
        .into_response())
}

async fn get_batch_job_error_csv_handler(
    batch_job_id: BatchJobId,
    token: TokenParams,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    let claims = decode::<ShareableEntityClaims>(
        &token.token,
        &DecodingKey::from_secret(app_state.jwt_secret.expose_secret().as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| Report::new(errors::RestApiError::Unauthorized))?
    .claims;

    let detail = app_state
        .store
        .get_batch_job(claims.entity_id.into(), claims.tenant_id)
        .await
        .change_context(errors::RestApiError::StoreError)?;

    if detail.job.id != batch_job_id {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }

    let error_key = detail
        .job
        .error_output_key
        .ok_or(Report::new(errors::RestApiError::NotFound))?;

    let doc_id: StoredDocumentId = error_key
        .parse()
        .map_err(|_| Report::new(errors::RestApiError::StoreError))?;

    let data = app_state
        .object_store
        .retrieve(
            doc_id,
            Prefix::BatchJobErrorOutput {
                tenant_id: claims.tenant_id,
            },
        )
        .await
        .change_context(errors::RestApiError::ObjectStoreError)?;

    let filename = format!("errors-{}.csv", batch_job_id);

    Ok((
        StatusCode::OK,
        [
            ("Content-Type", "text/csv; charset=utf-8"),
            (
                "Content-Disposition",
                &format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        data,
    )
        .into_response())
}
