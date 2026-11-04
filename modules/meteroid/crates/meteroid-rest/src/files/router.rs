use super::AppState;
use std::io::Cursor;

use axum::extract::Query;
use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use hyper::StatusCode;

use meteroid_app::errors;

use common_domain::ids::{
    BaseId, BatchJobId, CreditNoteId, CustomerId, EntityActivityId, InvoiceId, StoredDocumentId,
};
use error_stack::{Report, ResultExt};
use image::ImageFormat::Png;
use jsonwebtoken::{DecodingKey, Validation};
use meteroid_app::ledger_export;
use meteroid_app::services::storage;
use meteroid_app::sharable::ShareableEntityClaims;
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::domain::enums::StoredObjectKindEnum;
use meteroid_store::jwt::decode;
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
    uuid: StoredDocumentId,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    // Public, unauthenticated endpoint: `read_of_kind` guards that the id is actually a logo, so it
    // can't be used to fetch invoice PDFs or other kinds by id. Any failure surfaces as NotFound.
    let data = storage::read_of_kind(
        app_state.object_store.as_ref(),
        &app_state.store,
        uuid,
        StoredObjectKindEnum::ImageLogo,
    )
    .await
    .map_err(|_| Report::new(errors::RestApiError::NotFound))?;

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
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
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
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
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

/// The structured e-invoice (EN 16931 XML) as a download named after the document.
///
/// Stored beside the PDF rather than only inside it: a hybrid format embeds the same
/// bytes, but a receiver's import, an accountant, or a Peppol transport wants the
/// XML on its own.
pub fn xml_download(data: bytes::Bytes, document_number: &str) -> Response {
    let filename: String = document_number
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    (
        StatusCode::OK,
        [
            ("Content-Type", "application/xml".to_string()),
            (
                "Content-Disposition",
                format!("attachment; filename=\"{filename}.xml\""),
            ),
        ],
        data,
    )
        .into_response()
}

/// No XML on file: e-invoicing was off, the document could not be issued as one
/// (its findings say why), or generation has not run yet.
pub fn no_xml() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(RestErrorResponse {
            code: ErrorCode::NotFound,
            message: "No e-invoice XML: e-invoicing is off, the document was not issued as \
                      an e-invoice, or generation is pending"
                .to_string(),
        }),
    )
        .into_response()
}

#[axum::debug_handler]
pub async fn get_invoice_xml(
    Path(uid): Path<InvoiceId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_invoice_xml_handler(uid, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling invoice XML: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_invoice_xml_handler(
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
    match invoice.xml_document_id {
        Some(id) => {
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
                .await
                .change_context(errors::RestApiError::ObjectStoreError)?;
            Ok(xml_download(data, &invoice.invoice_number))
        }
        None => Ok(no_xml()),
    }
}

#[axum::debug_handler]
pub async fn get_credit_note_xml(
    Path(uid): Path<CreditNoteId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_credit_note_xml_handler(uid, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling credit note XML: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_credit_note_xml_handler(
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
    match credit_note.xml_document_id {
        Some(id) => {
            let data = storage::read(app_state.object_store.as_ref(), &app_state.store, id)
                .await
                .change_context(errors::RestApiError::ObjectStoreError)?;
            Ok(xml_download(data, &credit_note.credit_note_number))
        }
        None => Ok(no_xml()),
    }
}

/// Audit attachment shape stored in `sent_email.attachments`. `id` is absent on rows written
/// before attachment object ids were tracked.
#[derive(Deserialize)]
struct StoredEmailAttachment {
    filename: String,
    id: Option<String>,
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

    let data = storage::read(app_state.object_store.as_ref(), &app_state.store, doc_id)
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

    let data = storage::read(app_state.object_store.as_ref(), &app_state.store, doc_id)
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

    let data = storage::read(app_state.object_store.as_ref(), &app_state.store, doc_id)
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

#[axum::debug_handler]
pub async fn get_batch_job_output(
    Path(batch_job_id): Path<BatchJobId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_batch_job_output_handler(batch_job_id, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling batch job output: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_batch_job_output_handler(
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

    let output_key = detail
        .job
        .output_key
        .ok_or(Report::new(errors::RestApiError::NotFound))?;

    let doc_id: StoredDocumentId = output_key
        .parse()
        .map_err(|_| Report::new(errors::RestApiError::StoreError))?;

    let data = storage::read(app_state.object_store.as_ref(), &app_state.store, doc_id)
        .await
        .change_context(errors::RestApiError::ObjectStoreError)?;

    let filename = format!("tax-report-transactions-{}.csv", batch_job_id);

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

#[axum::debug_handler]
pub async fn get_credit_ledger_csv(
    Path(customer_id): Path<CustomerId>,
    Query(params): Query<TokenParams>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    match get_credit_ledger_csv_handler(customer_id, params, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling credit ledger CSV: {e:?}");
            e.current_context().clone().into_response()
        }
    }
}

async fn get_credit_ledger_csv_handler(
    customer_id: CustomerId,
    token: TokenParams,
    app_state: AppState,
) -> Result<Response, Report<errors::RestApiError>> {
    let scope = ledger_export::verify(&token.token, &app_state.jwt_secret)
        .ok_or_else(|| Report::new(errors::RestApiError::Unauthorized))?;
    if scope.customer_id != customer_id {
        return Err(Report::new(errors::RestApiError::Forbidden));
    }
    ledger_export::check_scope(&app_state.store, &scope)
        .await
        .change_context(errors::RestApiError::NotFound)?;

    Ok((
        StatusCode::OK,
        [
            ("Content-Type", "text/csv; charset=utf-8".to_string()),
            (
                "Content-Disposition",
                format!(
                    "attachment; filename=\"credit-ledger-{}.csv\"",
                    customer_id.as_base62()
                ),
            ),
        ],
        axum::body::Body::from_stream(ledger_export::csv_stream(app_state.store.clone(), scope)),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::xml_download;

    /// Named after the document so a downloaded XML is identifiable next to its PDF,
    /// with anything that could break out of the header's quoted filename replaced.
    #[test]
    fn the_xml_download_is_an_xml_attachment_named_after_the_document() {
        let response = xml_download(bytes::Bytes::from_static(b"<Invoice/>"), "INV-0099");
        let headers = response.headers();
        assert_eq!(headers["content-type"], "application/xml");
        assert_eq!(
            headers["content-disposition"],
            "attachment; filename=\"INV-0099.xml\""
        );

        let response = xml_download(bytes::Bytes::new(), "INV \"2026\"/42");
        assert_eq!(
            response.headers()["content-disposition"],
            "attachment; filename=\"INV__2026__42.xml\""
        );
    }
}
